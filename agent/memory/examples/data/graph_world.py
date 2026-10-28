"""graph_qa.json: a synthetic operations world as ideal extractions, and the questions labelled with the facts that
answer them (see make_datasets.py)."""

import random

# ---------------------------------------------------------------------------------------------------------------
# graph_qa.json
# ---------------------------------------------------------------------------------------------------------------

# name, symptom entity, issue (lesson), cue, symptom clause used in questions (never names the component), trivia
COMPONENTS = [
    ("Redis 7.2", "idle connection drops",
     "Redis 7.2 closes client connections that stay idle for 300 seconds; setting tcp-keepalive 60 in redis.conf keeps pooled connections open.",
     "pooled cache connections drop after idle minutes",
     "keeps losing its pooled cache connections after a few idle minutes",
     ["Redis 7.2 supports ACL users with per-command permissions.",
      "Redis 7.2 persists data with both RDB snapshots and the append-only file.",
      "Redis 7.2 cluster mode shards keys across 16384 hash slots."]),
    ("PostgreSQL 16", "connection limit reached",
     "PostgreSQL 16 refuses new connections once max_connections (100) is reached; putting PgBouncer in transaction mode in front of it stops the 'too many clients' errors.",
     "too many clients errors from the database",
     "fails with 'too many clients already' errors under load",
     ["PostgreSQL 16 adds logical replication from standby servers.",
      "PostgreSQL 16 stores JSON documents in jsonb columns with GIN indexes.",
      "PostgreSQL 16 nightly backups run with pg_dump in custom format."]),
    ("Kafka 3.6", "consumer rebalance loop",
     "Kafka 3.6 consumers rebalance in a loop when processing a batch takes longer than max.poll.interval.ms; raising it to 600000 or lowering max.poll.records ends the loop.",
     "consumers rebalance over and over",
     "keeps rebalancing its consumers and never makes progress",
     ["Kafka 3.6 runs without ZooKeeper in KRaft mode.",
      "Kafka 3.6 topics here keep messages for seven days.",
      "Kafka 3.6 producers use idempotence to avoid duplicate writes."]),
    ("Elasticsearch 8.11", "read-only index block",
     "Elasticsearch 8.11 marks indices read-only when disk usage passes the 95 percent flood-stage watermark; free disk space, then clear index.blocks.read_only_allow_delete.",
     "index writes rejected as read-only",
     "suddenly rejects every write with a read-only index error",
     ["Elasticsearch 8.11 enables security and TLS by default.",
      "Elasticsearch 8.11 snapshots go to an S3 repository every night.",
      "Elasticsearch 8.11 indices here use three primary shards."]),
    ("nginx 1.25", "request entity too large",
     "nginx 1.25 rejects request bodies over 1 MB with HTTP 413; raise client_max_body_size in the server block of the upload endpoints.",
     "uploads fail with HTTP 413",
     "returns HTTP 413 when users upload files bigger than a megabyte",
     ["nginx 1.25 supports HTTP/3 over QUIC.",
      "nginx 1.25 access logs are shipped to Loki.",
      "nginx 1.25 terminates TLS with certificates from Let's Encrypt."]),
    ("Node.js 20", "self-signed certificate rejected",
     "Node.js 20 rejects the internal self-signed certificates in fetch calls; point NODE_EXTRA_CA_CERTS at the company CA bundle instead of turning TLS checks off.",
     "outgoing HTTPS calls fail on the internal certificate",
     "fails outgoing HTTPS calls with 'self-signed certificate in certificate chain'",
     ["Node.js 20 includes a stable built-in test runner.",
      "Node.js 20 is the LTS line the frontend builds use.",
      "Node.js 20 runs ES modules natively."]),
    ("Python 3.12", "distutils missing",
     "Python 3.12 no longer ships distutils; installing setuptools restores the import for legacy setup.py builds.",
     "build fails with No module named distutils",
     "breaks its build with 'No module named distutils'",
     ["Python 3.12 improves error messages for common mistakes.",
      "Python 3.12 virtual environments are created with python -m venv.",
      "Python 3.12 type parameter syntax declares generic classes without TypeVar."]),
    ("OpenSSL 3.0", "legacy algorithm disabled",
     "OpenSSL 3.0 disables MD4 and other legacy algorithms; NTLM authentication works again after the legacy provider is enabled in openssl.cnf.",
     "NTLM authentication broke after a TLS library upgrade",
     "can no longer authenticate against the Windows NTLM proxy",
     ["OpenSSL 3.0 introduced the provider architecture.",
      "OpenSSL 3.0 is licensed under Apache 2.0.",
      "OpenSSL 3.0 ships a separately validated FIPS module."]),
    ("RabbitMQ 3.12", "memory alarm blocks publishers",
     "RabbitMQ 3.12 blocks every publisher when memory passes vm_memory_high_watermark (0.4 of RAM); add consumers or raise the watermark to clear the alarm.",
     "publishing hangs during traffic peaks",
     "hangs when publishing messages during traffic peaks",
     ["RabbitMQ 3.12 quorum queues replicate with Raft.",
      "RabbitMQ 3.12 management UI listens on port 15672.",
      "RabbitMQ 3.12 streams keep an append-only log of messages."]),
    ("MongoDB 7.0", "sort memory limit exceeded",
     "MongoDB 7.0 aborts queries whose in-memory sort exceeds 100 MB; add an index on the sort key or pass allowDiskUse.",
     "queries abort with a sort memory limit error",
     "aborts its report queries with a sort memory limit error",
     ["MongoDB 7.0 supports queryable encryption.",
      "MongoDB 7.0 replica sets here have three members.",
      "MongoDB 7.0 time-series collections store metrics compactly."]),
    ("Kubernetes 1.29", "ephemeral storage eviction",
     "Kubernetes 1.29 evicts pods whose ephemeral storage use exceeds their limit; set ephemeral-storage requests and limits on containers that write large logs.",
     "pods evicted for local storage use",
     "has its pods evicted with 'ephemeral local storage usage exceeds' messages",
     ["Kubernetes 1.29 clusters here run on three node pools.",
      "Kubernetes 1.29 sidecar containers start before the app containers.",
      "Kubernetes 1.29 workloads are applied with Argo CD."]),
    ("gRPC 1.60", "message size exceeded",
     "gRPC 1.60 rejects messages larger than 4 MB with RESOURCE_EXHAUSTED; raise the max receive message length on both client and server.",
     "large responses fail with RESOURCE_EXHAUSTED",
     "fails large responses with RESOURCE_EXHAUSTED errors",
     ["gRPC 1.60 supports xDS-based load balancing.",
      "gRPC 1.60 services define their APIs in protobuf files.",
      "gRPC 1.60 health checks use the standard health service."]),
    ("Celery 5.3", "duplicate task execution",
     "Celery 5.3 runs a task again when it outlives the broker's visibility_timeout; set visibility_timeout above the longest task duration.",
     "long background jobs run twice",
     "runs some long background jobs twice",
     ["Celery 5.3 schedules periodic jobs with celery beat.",
      "Celery 5.3 workers here run with a concurrency of eight.",
      "Celery 5.3 stores task results in the database backend."]),
    ("React 18", "effects run twice",
     "React 18 runs effects twice in development under StrictMode; make effects idempotent with cleanup functions instead of removing StrictMode.",
     "events fire twice in development",
     "fires its analytics events twice in development",
     ["React 18 batches state updates automatically.",
      "React 18 supports streaming server rendering with Suspense.",
      "React 18 components here are written as function components with hooks."]),
    ("Webpack 5", "missing Node polyfills",
     "Webpack 5 no longer polyfills Node core modules; add resolve.fallback entries for crypto, stream and buffer.",
     "bundling fails on a Node core module",
     "fails to bundle with 'Module not found: Can't resolve crypto'",
     ["Webpack 5 shares code between builds with Module Federation.",
      "Webpack 5 persistent caching stores build results on disk.",
      "Webpack 5 config here lives in webpack.config.js."]),
    ("Terraform 1.6", "stale state lock",
     "Terraform 1.6 leaves the state lock held after a crashed apply; run terraform force-unlock with the lock ID once no apply is running.",
     "plan blocked by a state lock",
     "cannot run a plan because the state is still locked after a crashed apply",
     ["Terraform 1.6 adds the terraform test command.",
      "Terraform 1.6 state is stored in an S3 backend.",
      "Terraform 1.6 modules here are pinned to exact versions."]),
    ("Docker 24", "build cache fills disk",
     "Docker 24 build cache grows until the CI runners run out of disk; schedule docker builder prune with an age filter every week.",
     "CI runners run out of disk",
     "makes its CI runners run out of disk space",
     ["Docker 24 builds images with BuildKit by default.",
      "Docker 24 images here are pushed to the internal Harbor registry.",
      "Docker 24 containers run as a non-root user."]),
    ("Spring Boot 3.2", "virtual thread pinning",
     "Spring Boot 3.2 with virtual threads pins carrier threads inside synchronized blocks and throughput collapses; replace those blocks with ReentrantLock.",
     "throughput collapses with virtual threads",
     "stalls under load since virtual threads were turned on",
     ["Spring Boot 3.2 supports the RestClient interface.",
      "Spring Boot 3.2 applications here run on Java 21.",
      "Spring Boot 3.2 exposes metrics through Micrometer."]),
    ("Go 1.22", "glibc version mismatch",
     "Go 1.22 binaries built on the new CI image fail with 'GLIBC_2.34 not found' on older hosts; build with CGO_ENABLED=0 to get static binaries.",
     "binary fails on older hosts with a glibc error",
     "crashes on older hosts with 'GLIBC_2.34 not found'",
     ["Go 1.22 range loops create a new variable per iteration.",
      "Go 1.22 ServeMux matches routes by method and path pattern.",
      "Go 1.22 modules here are vendored for reproducible builds."]),
    ("Envoy 1.28", "idle stream reset",
     "Envoy 1.28 resets HTTP/2 streams that stay idle for five minutes, which cuts long-polling requests; raise stream_idle_timeout on the route.",
     "long-polling requests cut at five minutes",
     "cuts off long-polling requests after exactly five minutes",
     ["Envoy 1.28 is configured through xDS from the control plane.",
      "Envoy 1.28 emits access logs in JSON format.",
      "Envoy 1.28 supports WebAssembly filters."]),
    ("Vault 1.15", "token expiry",
     "Vault 1.15 service tokens expire at the end of their TTL and calls start failing with 403; use periodic tokens and renew them from the service.",
     "secret store returns 403 after a few days",
     "starts getting 403 errors from its secret store after a few days",
     ["Vault 1.15 stores data in integrated Raft storage.",
      "Vault 1.15 is unsealed automatically with a cloud KMS key.",
      "Vault 1.15 audit logs go to a file device."]),
    ("Grafana 10", "provisioned dashboard overwritten",
     "Grafana 10 discards dashboard edits made in the UI when the dashboard is provisioned from files; change the JSON in the repository instead.",
     "dashboard edits lost on restart",
     "loses its dashboard changes after every restart",
     ["Grafana 10 supports public dashboards.",
      "Grafana 10 alerts are routed through notification policies.",
      "Grafana 10 reads metrics from Prometheus and logs from Loki."]),
    ("GitLab Runner 16", "job timeout",
     "GitLab Runner 16 kills jobs after the default one-hour timeout; set a longer timeout in the project CI settings for the integration suites.",
     "integration jobs killed at one hour",
     "has its integration test jobs killed after exactly one hour",
     ["GitLab Runner 16 uses the Docker executor here.",
      "GitLab Runner 16 caches dependencies in an S3 bucket.",
      "GitLab Runner 16 runners are registered with authentication tokens."]),
    ("Memcached 1.6", "item size limit",
     "Memcached 1.6 refuses items larger than 1 MB; raise the item size limit with -I 4m or split large values.",
     "large pages silently not cached",
     "silently fails to cache large rendered pages",
     ["Memcached 1.6 supports TLS between clients and servers.",
      "Memcached 1.6 instances here have 4 GB of memory each.",
      "Memcached 1.6 uses the meta protocol for new clients."]),
]

# service, [(component, purpose)], trivia parameters (day, p95 ms, log days, rotation)
SERVICES = [
    ("billing-api", [("Redis 7.2", "its session cache"), ("PostgreSQL 16", "its ledger database"), ("Kafka 3.6", "invoice events")], ("Tuesday", 250, 30, "payments")),
    ("checkout-web", [("Node.js 20", "server-side rendering"), ("React 18", "its user interface"), ("nginx 1.25", "its reverse proxy")], ("Wednesday", 300, 14, "storefront")),
    ("auth-service", [("Vault 1.15", "its signing keys"), ("OpenSSL 3.0", "TLS and NTLM"), ("Go 1.22", "its implementation")], ("Monday", 120, 90, "identity")),
    ("search-indexer", [("Elasticsearch 8.11", "its search index"), ("Kafka 3.6", "its change feed"), ("Python 3.12", "its indexing jobs")], ("Thursday", 800, 7, "search")),
    ("mobile-gateway", [("Envoy 1.28", "its edge proxy"), ("gRPC 1.60", "backend calls"), ("Kubernetes 1.29", "hosting")], ("Friday", 200, 30, "mobile")),
    ("data-pipeline", [("Celery 5.3", "its job queue"), ("Python 3.12", "its transforms"), ("MongoDB 7.0", "the raw event store")], ("Monday", 2000, 7, "data")),
    ("notification-worker", [("RabbitMQ 3.12", "its message queue"), ("Node.js 20", "the worker runtime"), ("Redis 7.2", "rate limits")], ("Thursday", 500, 14, "messaging")),
    ("admin-portal", [("React 18", "its user interface"), ("Webpack 5", "bundling"), ("nginx 1.25", "static hosting")], ("Tuesday", 400, 30, "internal-tools")),
    ("inventory-api", [("PostgreSQL 16", "its stock database"), ("Spring Boot 3.2", "its framework"), ("gRPC 1.60", "its internal API")], ("Wednesday", 150, 30, "catalog")),
    ("reporting-batch", [("MongoDB 7.0", "report queries"), ("Grafana 10", "its dashboards"), ("Memcached 1.6", "the rendered page cache")], ("Sunday", 5000, 60, "data")),
    ("edge-proxy", [("Envoy 1.28", "routing"), ("Docker 24", "image builds"), ("Terraform 1.6", "its infrastructure")], ("Friday", 50, 7, "platform")),
    ("chat-service", [("Kubernetes 1.29", "hosting"), ("RabbitMQ 3.12", "message fan-out"), ("GitLab Runner 16", "its CI pipeline")], ("Thursday", 180, 14, "messaging")),
]

TEAMS = [
    ("payments team", ["billing-api", "checkout-web"]),
    ("identity team", ["auth-service", "mobile-gateway"]),
    ("search team", ["search-indexer", "data-pipeline"]),
    ("messaging team", ["notification-worker", "chat-service"]),
    ("catalog team", ["inventory-api", "admin-portal"]),
    ("platform team", ["edge-proxy", "reporting-batch"]),
]

# vendor, [(product, office)]
VENDORS = [
    ("TP-Link", [("Archer C7", "Hanoi"), ("Deco M5", "Hue")]),
    ("Netgear", [("Nighthawk R7000", "Da Nang"), ("Orbi RBR50", "Can Tho")]),
    ("Cisco", [("Catalyst 9300", "Ho Chi Minh City"), ("Meraki MR46", "Hai Phong")]),
    ("Ubiquiti", [("UniFi Dream Machine", "Nha Trang"), ("UniFi U6-LR", "Vung Tau")]),
]

# symptom entity, question template ({p} = product), cue, {vendor: lesson}
NET_SYMPTOMS = [
    ("DHCP lease drops", "DHCP leases keep dropping on the {p}. What should we change?", "DHCP leases drop on a router", {
        "TP-Link": "On TP-Link routers, DHCP leases drop after a firmware update resets the lease table; reserving the addresses under Advanced > Network > DHCP Server keeps clients stable.",
        "Netgear": "On Netgear routers, DHCP leases drop when Smart Connect moves clients between bands; turning Smart Connect off keeps the leases.",
        "Cisco": "On Cisco devices, DHCP leases drop when DHCP snooping treats the uplink as untrusted; mark the uplink port with ip dhcp snooping trust.",
        "Ubiquiti": "On Ubiquiti gateways, DHCP leases drop when DHCP guarding blocks the server; add the gateway address to the trusted DHCP servers list.",
    }),
    ("5 GHz network disappears", "The 5 GHz Wi-Fi network vanishes every few hours on the {p}. How do we keep it up?", "the 5 GHz Wi-Fi network disappears", {
        "TP-Link": "On TP-Link routers, the 5 GHz network disappears when auto channel picks a DFS channel; set a fixed channel 36 in the wireless settings.",
        "Netgear": "On Netgear routers, the 5 GHz network disappears after radar detection on DFS channels; disable DFS channels in the advanced wireless settings.",
        "Cisco": "On Cisco access points, the 5 GHz radio shuts down when the RF profile sets the minimum bitrate too high; lower it to 12 Mbps in the RF profile.",
        "Ubiquiti": "On Ubiquiti access points, the 5 GHz network disappears when band steering conflicts with older clients; turn band steering off for that SSID.",
    }),
    ("VPN tunnel drops", "The site-to-site VPN tunnel through the {p} drops every few hours. What fixes it?", "site-to-site VPN tunnel drops periodically", {
        "TP-Link": "On TP-Link routers, the site-to-site VPN drops at rekey when IKE lifetimes differ; set the IKE lifetime to 28800 seconds on both ends.",
        "Netgear": "On Netgear routers, the site-to-site VPN drops after an idle period; enable dead peer detection with a 30 second interval.",
        "Cisco": "On Cisco devices, the site-to-site VPN drops at phase 2 rekey when PFS groups differ; use the same PFS group on both peers.",
        "Ubiquiti": "On Ubiquiti gateways, the site-to-site VPN drops when WAN failover moves it to the backup link; pin the tunnel to the WAN1 interface.",
    }),
    ("firmware update failure", "A firmware update on the {p} fails halfway. How do we complete it safely?", "a firmware update fails halfway", {
        "TP-Link": "On TP-Link routers, firmware updates fail over Wi-Fi; upload the image through the web UI from a wired computer.",
        "Netgear": "On Netgear routers, automatic firmware updates fail halfway on slow links; download the image from the support site and use manual upload.",
        "Cisco": "On Cisco devices, firmware installs fail when the image is corrupt; copy it to flash with SCP and verify the MD5 hash before installing.",
        "Ubiquiti": "On Ubiquiti devices, firmware updates fail when the UniFi controller disk is full; free controller disk space, then update from the controller.",
    }),
    ("guest network isolation", "Guest Wi-Fi clients can reach internal servers through the {p}. How do we isolate them?", "guest Wi-Fi clients reach the internal network", {
        "TP-Link": "On TP-Link routers, guests reach the LAN when 'Allow guests to access my local network' is on; turn that option off in the guest network settings.",
        "Netgear": "On Netgear routers, guests reach the LAN unless guest isolation is enabled; tick 'Enable guest network isolation' for the guest SSID.",
        "Cisco": "On Cisco networks, guest clients reach internal hosts unless a group policy denies LAN access; apply a guest group policy that blocks private ranges.",
        "Ubiquiti": "On Ubiquiti networks, guests reach the LAN unless client device isolation is on; enable it and add a traffic rule blocking local networks.",
    }),
]


def graph_qa():
    facts = []

    def fact(key, text, kind="fact", cue=None, scope=None, entities=(), relations=()):
        f = {"key": key, "text": text, "kind": kind, "entities": list(entities), "relations": list(relations)}
        if cue:
            f["cue"] = cue
        if scope:
            f["scope"] = scope
        facts.append(f)

    def ent(name, typ, role="mention", scope=None):
        e = {"name": name, "type": typ, "role": role}
        if scope:
            e["scope"] = scope
        return e

    comp = {c[0]: c for c in COMPONENTS}
    for name, symptom, issue, cue, _clause, trivia in COMPONENTS:
        fact(f"issue:{name}", issue, "lesson", cue,
             entities=[ent(name, "software", "subject"), ent(symptom, "error", "object")],
             relations=[{"src": name, "type": "causes", "dst": symptom}])
        for i, t in enumerate(trivia):
            fact(f"trivia:{name}:{i}", t, entities=[ent(name, "software", "subject")])
    for svc, uses, (day, p95, logs, rot) in SERVICES:
        for c, purpose in uses:
            assert c in comp, c
            fact(f"uses:{svc}:{c}", f"{svc} uses {c} for {purpose}.",
                 entities=[ent(svc, "service", "subject"), ent(c, "software", "object")],
                 relations=[{"src": svc, "type": "uses", "dst": c}])
        for i, t in enumerate([
            f"{svc} deploys to production every {day} after the change review.",
            f"{svc} has a p95 latency budget of {p95} ms.",
            f"{svc} writes structured JSON logs that are kept for {logs} days.",
            f"{svc} pages the on-call engineer through the {rot} rotation.",
        ]):
            fact(f"svc:{svc}:{i}", t, entities=[ent(svc, "service", "subject")])
    for team, svcs in TEAMS:
        fact(f"team:{team}", f"The {team} owns {svcs[0]} and {svcs[1]}.",
             entities=[ent(team, "team", "subject")] + [ent(s, "service", "object") for s in svcs],
             relations=[{"src": s, "type": "part_of", "dst": team} for s in svcs])
    for vendor, products in VENDORS:
        for p, office in products:
            fact(f"product:{p}", f"The {p} is the {vendor} device in the {office} office.",
                 entities=[ent(p, "product", "subject", {"vendor": vendor}), ent(vendor, "vendor", "object")],
                 relations=[{"src": p, "type": "part_of", "dst": vendor}])
    for symptom, _q, cue, by_vendor in NET_SYMPTOMS:
        for vendor, text in by_vendor.items():
            fact(f"net:{vendor}:{symptom}", text, "lesson", cue, {"vendor": vendor},
                 entities=[ent(vendor, "vendor", "subject"), ent(symptom, "error", "object")],
                 relations=[{"src": symptom, "type": "applies_to", "dst": vendor}])

    questions = []

    def q(typ, text, gold, forbidden=()):
        questions.append({"id": f"Q{len(questions) + 1:03}", "type": typ, "text": text, "gold": [gold],
                          "forbidden": list(forbidden)})

    for name, _s, _i, _c, clause, _t in COMPONENTS:
        q("single-hop", f"Our {name} {clause}. How do we fix it?", f"issue:{name}")
    for svc, uses, _ in SERVICES:
        for c, _p in uses:
            q("two-hop", f"{svc} {clause_of(comp, c)}. How do we fix it?", f"issue:{c}")
    svc_uses = {s: [c for c, _ in u] for s, u, _ in SERVICES}
    for team, svcs in TEAMS:
        # One component per service of the team, the first one whose symptom no other team component shares.
        for s in svcs:
            c = svc_uses[s][0]
            q("two-hop-team", f"A service owned by the {team} {clause_of(comp, c)}. How do we fix it?", f"issue:{c}")
    for symptom, template, _cue, by_vendor in NET_SYMPTOMS:
        for vendor, products in VENDORS:
            for p, _office in products:
                q("scope-trap", template.format(p=p), f"net:{vendor}:{symptom}",
                  [f"net:{v}:{symptom}" for v in by_vendor if v != vendor])

    keys = [f["key"] for f in facts]
    assert len(keys) == len(set(keys))
    texts = [f["text"] for f in facts]
    assert len(texts) == len(set(texts))
    for x in questions:
        assert all(k in keys for k in x["gold"] + x["forbidden"]), x

    # Episodes of six facts in a fixed shuffled order (resolution sees the same entities across episodes).
    order = list(range(len(facts)))
    random.Random(51).shuffle(order)
    episodes = []
    for i in range(0, len(order), 6):
        episodes.append({"source": f"ticket:ACME-{i // 6 + 1}", "facts": [facts[j] for j in order[i:i + 6]]})
    return {
        "about": "Synthetic operations world as ideal extractions, and questions labelled with the fact that answers "
                 "them. Generated by make_datasets.py; evaluated by graph_qa.rs.",
        "project": "acme",
        "facts": len(facts),
        "episodes": episodes,
        "questions": questions,
    }


def clause_of(comp, c):
    return comp[c][4]


# ---------------------------------------------------------------------------------------------------------------
# entity_pairs.json
# ---------------------------------------------------------------------------------------------------------------
