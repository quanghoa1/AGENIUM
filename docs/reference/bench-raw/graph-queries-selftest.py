# Self-test of the ADR-51 example queries on a 7-entity graph (SQLite >= 3.37). Run: python3 graph-queries-selftest.py
import sqlite3, os
HERE=os.path.dirname(os.path.abspath(__file__))
db=sqlite3.connect(':memory:')
db.executescript(open(os.path.join(HERE,'../../crates/agenium-store/schema/sqlite/agent_001.sql')).read())
db.executescript(open(os.path.join(HERE,'../../schemas/sql/drafts/agent_003_graph_memory.sql')).read())
E=lambda t,n,s='{}',sk='':db.execute("INSERT INTO entity(type,name,canonical_key,scope,scope_key) VALUES(?,?,?,?,?)",(t,n,n.lower(),s,sk)).lastrowid
tp=E('vendor','TP-Link'); ng=E('vendor','Netgear'); ax=E('device','Archer AX50','{"vendor":"tp-link"}','vendor=tp-link')
fw=E('software','firmware 1.2'); bug=E('error','DHCP lease drop'); wk=E('config','disable QoS offload'); cyc=E('concept','loop')
R=lambda s,d,t,sc='{}',sk='',c=0.8:db.execute("INSERT INTO edge(src,dst,type,scope,scope_key,confidence) VALUES(?,?,?,?,?,?)",(s,d,t,sc,sk,c))
R(ax,tp,'part_of'); R(ax,fw,'uses'); R(fw,bug,'causes'); R(wk,bug,'workaround_for','{"vendor":"tp-link"}','vendor=tp-link'); R(bug,fw,'related_to'); R(fw,cyc,'related_to'); R(cyc,ax,'related_to')
f=lambda t,k,sc='{}':db.execute("INSERT INTO fact(text,kind,scope,source) VALUES(?,?,?,'ticket:AGN-1')",(t,k,sc)).lastrowid
f1=f('On TP-Link Archer firmware 1.2 DHCP leases drop; disable QoS offload.','lesson','{"vendor":"tp-link"}')
f2=f('Netgear needs a reboot after DHCP change.','lesson','{"vendor":"netgear"}')
for fid,eid in((f1,ax),(f1,bug),(f1,wk),(f2,ng)): db.execute("INSERT INTO fact_entity(fact_id,entity_id,role) VALUES(?,?, 'mention')",(fid,eid))
SC="NOT EXISTS (SELECT 1 FROM json_each({a}.scope) s WHERE json_extract(:ctx,'$.'||s.key) IS NOT NULL AND json_extract(:ctx,'$.'||s.key) <> s.value)"
Q1=f"""WITH RECURSIVE walk(node,depth,score,path) AS (
  SELECT :seed,0,1.0,','||:seed||','
  UNION ALL
  SELECT x.dst,w.depth+1,w.score*t.hop_weight*x.confidence,w.path||x.dst||','
  FROM walk w JOIN edge x INDEXED BY edge_out ON x.src=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:max_depth AND t.traverse=1 AND instr(w.path,','||x.dst||',')=0 AND {SC.format(a='x')}
  UNION ALL
  SELECT x.src,w.depth+1,w.score*t.hop_weight*x.confidence,w.path||x.src||','
  FROM walk w JOIN edge x INDEXED BY edge_in ON x.dst=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:max_depth AND t.traverse=1 AND instr(w.path,','||x.src||',')=0 AND {SC.format(a='x')})
SELECT node,MAX(score) AS score,MIN(depth) AS hops FROM walk WHERE depth>0 GROUP BY node ORDER BY score DESC LIMIT 50"""
for ctx in ('{"vendor":"tp-link"}','{"vendor":"netgear"}'):
    print(ctx, db.execute(Q1,{'seed':ax,'max_depth':3,'ctx':ctx}).fetchall())
# facts reachable from seed set, scope-filtered, hop-decayed
Q2=f"""WITH RECURSIVE walk(node,depth,score,path) AS (
  SELECT value,0,1.0,','||value||',' FROM json_each(:seeds)
  UNION ALL
  SELECT x.dst,w.depth+1,w.score*t.hop_weight*x.confidence,w.path||x.dst||','
  FROM walk w JOIN edge x INDEXED BY edge_out ON x.src=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:max_depth AND t.traverse=1 AND instr(w.path,','||x.dst||',')=0 AND {SC.format(a='x')}
  UNION ALL
  SELECT x.src,w.depth+1,w.score*t.hop_weight*x.confidence,w.path||x.src||','
  FROM walk w JOIN edge x INDEXED BY edge_in ON x.dst=w.node AND x.valid_to IS NULL JOIN edge_type t ON t.name=x.type
  WHERE w.depth<:max_depth AND t.traverse=1 AND instr(w.path,','||x.src||',')=0 AND {SC.format(a='x')}),
reach AS (SELECT node,MAX(score) AS score,MIN(depth) AS hops FROM walk GROUP BY node)
SELECT f.id, f.text, MAX(r.score*f.confidence) AS graph_score, MIN(r.hops) AS hops
FROM reach r JOIN fact_entity fe ON fe.entity_id=r.node JOIN fact f ON f.id=fe.fact_id
WHERE f.superseded_by IS NULL AND f.valid_to IS NULL AND {SC.format(a='f')}
GROUP BY f.id ORDER BY graph_score DESC LIMIT 30"""
import json
for ctx in ('{"vendor":"tp-link"}','{"vendor":"netgear"}'):
    print(ctx, db.execute(Q2,{'seeds':json.dumps([tp]),'max_depth':2,'ctx':ctx}).fetchall())
    print(ctx,'seed=bug', db.execute(Q2,{'seeds':json.dumps([bug]),'max_depth':1,'ctx':ctx}).fetchall())
# ancestors via is_a / part_of
Q3="""WITH RECURSIVE up(node,depth) AS (SELECT :e,0 UNION SELECT x.dst,u.depth+1 FROM up u JOIN edge x ON x.src=u.node AND x.type IN ('is_a','part_of') AND x.valid_to IS NULL WHERE u.depth<5)
SELECT node,MIN(depth) FROM up GROUP BY node"""
print(db.execute(Q3,{'e':ax}).fetchall())
# shortest path (<= 4 hops) between :a and :b, BFS by depth, first hit
Q4="""WITH RECURSIVE bfs(node,depth,path) AS (SELECT :a,0,','||:a||','
 UNION ALL SELECT x.dst,b.depth+1,b.path||x.dst||',' FROM bfs b JOIN edge x INDEXED BY edge_out ON x.src=b.node AND x.valid_to IS NULL
 WHERE b.depth<:max AND instr(b.path,','||x.dst||',')=0 AND b.node<>:b
 UNION ALL SELECT x.src,b.depth+1,b.path||x.src||',' FROM bfs b JOIN edge x INDEXED BY edge_in ON x.dst=b.node AND x.valid_to IS NULL
 WHERE b.depth<:max AND instr(b.path,','||x.src||',')=0 AND b.node<>:b)
SELECT path,depth FROM bfs WHERE node=:b ORDER BY depth LIMIT 1"""
print(db.execute(Q4,{'a':wk,'b':tp,'max':4}).fetchall())
