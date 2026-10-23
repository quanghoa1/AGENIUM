# Feasibility probe for ADR-51 (python sqlite3, NOT the Rust store). Usage: GRAPH_PROBE_DIR=/tmp python3 graph-cte-probe.py <edges>  (writes $GRAPH_PROBE_DIR/g<edges>.db)
import sqlite3, random, time, sys, json, os, statistics as st
HERE=os.path.dirname(os.path.abspath(__file__)); W=os.environ.get('GRAPH_PROBE_DIR','/tmp')
E=int(sys.argv[1]); N=max(E//3,10); random.seed(7)
db=sqlite3.connect(f'{W}/g{E}.db'); 
db.executescript("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")
sch=open(os.path.join(HERE,'../../schemas/sql/drafts/agent_003_graph_memory.sql')).read()
base=open(os.path.join(HERE,'../../crates/agenium-store/schema/sqlite/agent_001.sql')).read()
import os
BUILT=os.path.exists(f'{W}/g{E}.done')
if not BUILT: db.executescript(base); db.executescript(sch)
types=['is_a','part_of','depends_on','uses','causes','fixes','workaround_for','conflicts_with','applies_to','related_to']
t0=time.time()
if not BUILT:
    t0=time.time()
    db.executemany("INSERT INTO entity(id,type,name,canonical_key,scope_key) VALUES(?,?,?,?,?)",[(i,'concept',f'e{i}',f'e{i}','') for i in range(1,N+1)])
    rows=set()
    scopes=['{}']*9+['{"vendor":"tp-link"}']
    while len(rows)<E:
        a=random.randint(1,N); b=1+int(N*random.random()**2)
        if a==b: continue
        t=random.choice(types)
        rows.add((a,b,t))
    db.executemany("INSERT INTO edge(src,dst,type,scope,scope_key,confidence) VALUES(?,?,?,?,?,?)",[(a,b,t,(s:=random.choice(scopes)),'vendor=tp-link' if s!='{}' else '',round(random.uniform(.4,1),2)) for a,b,t in rows])
    db.commit(); db.execute("ANALYZE"); db.commit()
    print(f"edges={E} entities={N} build={time.time()-t0:.1f}s file={__import__('os').path.getsize(f'{W}/g{E}.db')/1e6:.0f}MB",flush=True)
    
    open(f'{W}/g{E}.done','w').write('x')
db.executescript("""
CREATE TEMP VIEW adj AS
 SELECT src AS a, dst AS b, type, confidence, scope, id FROM edge WHERE valid_to IS NULL
 UNION ALL SELECT dst, src, type, confidence, scope, id FROM edge WHERE valid_to IS NULL;
""")
SCOPE_OK="""NOT EXISTS (SELECT 1 FROM json_each(x.scope) s WHERE json_extract(:ctx,'$.'||s.key) IS NOT NULL AND json_extract(:ctx,'$.'||s.key) <> s.value)"""

QA=f"""
WITH RECURSIVE walk(node,depth,score,path) AS (
  SELECT :seed,0,1.0,','||:seed||','
  UNION ALL
  SELECT x.dst, w.depth+1, w.score*t.hop_weight*x.confidence, w.path||x.dst||','
  FROM walk w JOIN edge x INDEXED BY edge_out ON x.src=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:maxd AND t.traverse=1 AND instr(w.path,','||x.dst||',')=0 AND {SCOPE_OK}
  UNION ALL
  SELECT x.src, w.depth+1, w.score*t.hop_weight*x.confidence, w.path||x.src||','
  FROM walk w JOIN edge x INDEXED BY edge_in ON x.dst=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:maxd AND t.traverse=1 AND instr(w.path,','||x.src||',')=0 AND {SCOPE_OK})
SELECT node,MAX(score),MIN(depth) FROM walk WHERE depth>0 GROUP BY node ORDER BY 2 DESC LIMIT 50"""
QB=f"""
WITH RECURSIVE walk(node,depth) AS (
  SELECT :seed,0
  UNION
  SELECT x.dst, w.depth+1 FROM walk w JOIN edge x INDEXED BY edge_out ON x.src=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:maxd AND t.traverse=1 AND {SCOPE_OK}
  UNION
  SELECT x.src, w.depth+1 FROM walk w JOIN edge x INDEXED BY edge_in ON x.dst=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:maxd AND t.traverse=1 AND {SCOPE_OK})
SELECT node,MIN(depth) FROM walk WHERE depth>0 GROUP BY node LIMIT 50"""
def run(q,seeds,maxd,limit_s=3.0):
    lat=[];to=0;sizes=[]
    for s in seeds:
        t=time.time(); dl=[t+limit_s]
        db.set_progress_handler(lambda: 1 if time.time()>dl[0] else 0, 10000)
        try:
            r=db.execute(q,{'seed':s,'maxd':maxd,'ctx':'{"vendor":"netgear"}'}).fetchall(); sizes.append(len(r))
        except sqlite3.OperationalError: to+=1
        lat.append((time.time()-t)*1000)
    db.set_progress_handler(None,0)
    lat.sort(); return lat[len(lat)//2], lat[int(len(lat)*.95)-1], to, (st.mean(sizes) if sizes else 0)
seeds=[random.randint(1,N) for _ in range(100)]
hub=[1+int(N*random.random()**2) for _ in range(100)]   # likely hubs
for name,q in (("A path-guarded",QA),("B set-based UNION",QB)):
    for d in (1,2,3):
        for sn,sd in (("random",seeds),("hub-biased",hub)):
            sd2=sd if not(name[0]=='A' and d==3) else sd[:30]
            p50,p95,to,avg=run(q,sd2,d)
            print(f"{name:18s} depth={d} seeds={sn:10s} n={len(sd2)} p50={p50:8.2f}ms p95={p95:8.2f}ms timeouts(>3s)={to} avg_result_rows={avg:.0f}",flush=True)
