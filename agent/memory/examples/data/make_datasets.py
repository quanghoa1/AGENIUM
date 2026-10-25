"""Generates the two labelled memory datasets (ADR-51 §13 items 3-4), deterministically:

  graph_qa.json      a synthetic operations world (services, components with known issues, teams, network vendors and
                     products) as ideal extractions, plus >= 100 questions labelled with the fact that answers each
                     (single-hop, two-hop through a service, two-hop through a team, scope traps with the facts that
                     must NOT be shown)
  entity_pairs.json  a catalogue of entities as stored in memory, plus ~2 000 mentions labelled with the entity each one
                     names (or none): surface variants, aliases, sibling products, other versions, conflicting vendor
                     scopes, other types and outsiders

Labels come from construction, not from a model. Run from anywhere: python -X utf8 -I make_datasets.py
The evaluators are ../graph_qa.rs and ../entity_resolution.rs.
"""

import json
import random
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent

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

# type | name | vendor scope | generic noun | description | aliases | siblings (different entities, not stored)
CATALOGUE = """
software|PostgreSQL||database|open-source relational database server|Postgres;PG|PostGIS;PostgREST;Postgres-XL
software|MySQL||database|relational database server owned by Oracle|My SQL|MySQL Workbench;MySQL Router;MySQL Shell
software|MariaDB||database|community fork of the MySQL database|Maria DB|MariaDB MaxScale;MariaDB Galera;MariaDB Connector/J
software|Redis||cache|in-memory key-value store used as a cache and message broker|redis-server;Redis OSS|Redisson;RedisInsight;RedisJSON
software|Memcached||cache|distributed in-memory object cache|memcache|Memcachier;libmemcached;MemcacheDB
software|MongoDB||database|document-oriented NoSQL database|Mongo;Mongo DB|MongoDB Atlas;MongoDB Compass;Mongoose
software|Elasticsearch||search engine|distributed search and analytics engine|Elastic Search;ES|Elastic Agent;Elasticsearch Curator;OpenSearch
software|Kafka||message broker|distributed event streaming platform from Apache|Apache Kafka|Kafka Connect;Kafka Streams;Kafka UI
software|RabbitMQ||message broker|AMQP message broker|Rabbit MQ|RabbitMQ Streams;RabbitMQ Management Plugin;RabbitMQ Shovel
software|nginx||web server|HTTP server and reverse proxy|NGINX Open Source;engine x|NGINX Unit;ingress-nginx;NGINX Plus
software|Apache HTTP Server||web server|the Apache httpd web server|httpd;Apache httpd|Apache Tomcat;Apache Spark;Apache Traffic Server
software|Docker||container runtime|container engine and image tooling|Docker Engine;dockerd|Docker Compose;Docker Desktop;Docker Swarm
software|Kubernetes||orchestrator|container orchestration platform|k8s;kube|Kubeflow;Kubernetes Dashboard;KubeVirt
software|Terraform||IaC tool|infrastructure as code tool from HashiCorp|terraform cli;TF|Terragrunt;Terraform Cloud;OpenTofu
software|Vault||secret store|HashiCorp secrets manager|HashiCorp Vault|Vaultwarden;Ansible Vault;Vault Agent
software|Consul||service discovery|HashiCorp service discovery and mesh|HashiCorp Consul|Consul Template;Consul ESM;Consular
software|Grafana||dashboard|metrics dashboards and visualisation|Grafana OSS|Grafana Loki;Grafana Tempo;Grafana Agent
software|Prometheus||monitoring system|metrics collection and alerting toolkit|Prometheus server;prom|Prometheus Alertmanager;Prometheus Pushgateway;Thanos
software|Jenkins||CI server|automation server for builds|Jenkins CI|Jenkins X;Jenkinsfile Runner;Hudson
software|GitLab Runner||CI agent|agent that runs GitLab CI jobs|gitlab-runner|GitLab Pages;GitLab Workhorse;GitLab Shell
software|Envoy||proxy|L7 proxy and communication bus|Envoy Proxy|Envoy Gateway;Envoy Mobile;Emissary
software|Istio||service mesh|service mesh built on Envoy|Istio service mesh|Istio Ambient;Linkerd;Kiali
software|Celery||task queue|distributed task queue for Python|Celery workers|Celery Beat;Flower;Dramatiq
software|Sentry||error tracker|application error monitoring|sentry.io|Sentry Relay;Sentry CLI;Bugsnag
software|OpenSSL||crypto library|TLS and cryptography toolkit|openssl lib|LibreSSL;BoringSSL;OpenSSH
software|SQLite||database|embedded SQL database engine|sqlite3|SQLCipher;sqlite-vec;libSQL
software|ClickHouse||database|column-oriented analytics database|Click House|ClickHouse Keeper;chDB;ClickHouse Cloud
software|Cassandra||database|wide-column distributed database from Apache|Apache Cassandra|ScyllaDB;Cassandra Reaper;DataStax Enterprise
software|Airflow||workflow scheduler|Apache workflow orchestration platform|Apache Airflow|Airbyte;Argo Workflows;Astronomer
software|Argo CD||deployment tool|GitOps continuous delivery for Kubernetes|ArgoCD|Argo Rollouts;Argo Events;Flux CD
software|Helm||package manager|package manager for Kubernetes|helm cli|Helmfile;Helm Dashboard;Kustomize
software|Ansible||automation tool|agentless configuration management|Ansible Core|Ansible Tower;Ansible Lint;AWX
software|Webpack||bundler|JavaScript module bundler|webpack.js|Webpacker;Turbopack;webpack-dev-server
software|Vite||bundler|frontend build tool|ViteJS|Vitest;VitePress;Vike
software|React||UI library|JavaScript library for building user interfaces|ReactJS;React.js|React Native;Preact;React Router
software|Vue||UI framework|progressive JavaScript framework|Vue.js;VueJS|Vuex;Nuxt;Vuetify
software|Angular||UI framework|TypeScript web framework from Google|Angular.io|AngularJS;Angular Material;Angular Universal
software|Django||web framework|Python web framework|Django framework|Django REST framework;Django Channels;Flask
software|Spring Boot||framework|Java application framework|SpringBoot|Spring Cloud;Spring Batch;Spring Security
software|Express||web framework|minimal Node.js web framework|Express.js;ExpressJS|Fastify;Koa;Express Gateway
software|FastAPI||web framework|Python async API framework|Fast API|FastAPI Users;Starlette;FastStream
software|gRPC||RPC framework|high-performance RPC framework from Google|grpc framework|gRPC-Web;gRPC Gateway;tRPC
software|Keycloak||identity server|open-source identity and access management|Key Cloak|Keycloak Gatekeeper;Keystone;Authelia
software|Traefik||proxy|cloud-native reverse proxy|Traefik Proxy|Traefik Mesh;Traefik Hub;Traefik Pilot
software|HAProxy||load balancer|TCP and HTTP load balancer|HA Proxy|HAProxy Data Plane API;HAProxy ALOHA;HAProxy Fusion
software|Logstash||log processor|server-side log processing pipeline|Log Stash|Logstash Forwarder;Filebeat;Fluentd
language|Python||language|general-purpose programming language|py|Jython;MicroPython;Cython
language|JavaScript||language|scripting language of the web|JS;ECMAScript|JScript;CoffeeScript;JavaScriptCore
language|TypeScript||language|typed superset of JavaScript|TS|AssemblyScript;ReScript;PureScript
language|Java||language|object-oriented language running on the JVM|Java SE|Kotlin;JavaFX;Javalin
language|Go||language|programming language from Google|Golang|Go kit;Gogs;Goa
language|Rust||language|systems programming language|Rust lang;rustlang|Rust Analyzer;Rustls;Rocket
language|C#||language|Microsoft's object-oriented language|C Sharp;CSharp|C++;F#;C
language|Ruby||language|dynamic object-oriented language|Ruby lang|Ruby on Rails;RubyGems;Crystal
language|PHP||language|server-side scripting language|PHP lang|PHPUnit;PHP-FPM;Hack
language|Swift||language|Apple's programming language|Swift lang|SwiftUI;Swift Package Manager;Objective-C
language|Elixir||language|functional language on the Erlang VM|Elixir lang|Erlang;Phoenix;Elm
language|Scala||language|functional and object-oriented JVM language|Scala lang|Scala.js;ScalaTest;Akka
os|Ubuntu 22.04||OS|Ubuntu Linux LTS release Jammy Jellyfish|Jammy;Ubuntu Jammy|Kubuntu 22.04;Ubuntu Core 22;Ubuntu Pro
os|Windows 11||OS|Microsoft desktop operating system|Win11;Win 11|Windows Server 2022;Windows 11 IoT Enterprise;Windows 365
os|macOS Sonoma||OS|macOS 14 desktop operating system|macOS 14;Sonoma|macOS Ventura;macOS Sequoia;visionOS 1
os|Debian 12||OS|Debian Linux release Bookworm|Bookworm;Debian Bookworm|Devuan 5;Deepin 23;Raspberry Pi OS
os|Alpine Linux||OS|small security-oriented Linux distribution|Alpine|AlmaLinux;Arch Linux;Amazon Linux
os|Red Hat Enterprise Linux 9||OS|enterprise Linux distribution|RHEL 9|Rocky Linux 9;CentOS Stream 9;Oracle Linux 9
os|FreeBSD||OS|BSD Unix operating system|Free BSD|OpenBSD;NetBSD;DragonFly BSD
os|Android 14||OS|Google mobile operating system|Upside Down Cake|Android TV 14;Wear OS 4;Android Go
os|iOS 17||OS|Apple mobile operating system|iPhone OS 17|iPadOS 17;watchOS 10;tvOS 17
vendor|TP-Link||vendor|networking equipment maker|TPLink|Mercusys;Kasa Smart;D-Link
vendor|Netgear||vendor|networking hardware company|NETGEAR Inc.|Netgate;Arlo;NetApp
vendor|Cisco||vendor|networking and security company|Cisco Systems|Meraki;Sysco;Ciena
vendor|Ubiquiti||vendor|maker of UniFi networking gear|Ubiquiti Networks;UBNT|UniFi;Ubisoft;Uniview
vendor|Juniper Networks||vendor|network equipment vendor|Juniper|Juniper Research;Arista Networks;Extreme Networks
vendor|Microsoft||vendor|software company behind Windows and Azure|MSFT|Micron;Microchip;Microsemi
vendor|Amazon Web Services||vendor|cloud provider|AWS;Amazon AWS|Amazon;Azure;Amazon Prime Video
vendor|Google Cloud||vendor|Google's cloud platform|GCP;Google Cloud Platform|Google;Google Workspace;Cloudflare
vendor|HashiCorp||vendor|maker of Terraform and Vault|Hashicorp Inc|Harness;Pulumi;Hashnode
vendor|Atlassian||vendor|maker of Jira and Confluence|Atlassian Corp|Atlas Copco;Asana;Atlantic
vendor|Fortinet||vendor|network security company|Fortinet Inc|Fortanix;Forcepoint;Fortive
vendor|Dell||vendor|computer hardware company|Dell Technologies|Delta Electronics;HP;Lenovo
vendor|Synology||vendor|maker of NAS devices|Synology Inc|QNAP;Synopsys;Symantec
product|Archer C7|tp-link|router|TP-Link dual-band Wi-Fi router|AC1750|Archer C6;Archer A7;Archer C80
product|Deco M5|tp-link|mesh system|TP-Link whole-home mesh Wi-Fi|Deco M5 mesh|Deco X20;Deco M4;Deco E4
product|Nighthawk R7000|netgear|router|Netgear AC1900 router|R7000;Nighthawk AC1900|Nighthawk R7000P;Nighthawk R8000;Nighthawk RAX50
product|Orbi RBR50|netgear|mesh router|Netgear Orbi mesh base router|RBR50;Orbi AC3000|Orbi RBS50;Orbi RBR750;Orbi RBK50
product|Catalyst 9300|cisco|switch|Cisco stackable enterprise switch|C9300;Cat 9300|Catalyst 9200;Catalyst 9500;Catalyst 9300X
product|Meraki MR46|cisco|access point|Cisco Meraki Wi-Fi 6 access point|MR46|Meraki MR36;Meraki MR46E;Meraki MX68
product|UniFi Dream Machine|ubiquiti|gateway|Ubiquiti all-in-one gateway and controller|UDM;Dream Machine|UniFi Dream Machine Pro;UniFi Dream Router;UniFi Cloud Gateway
product|UniFi U6-LR|ubiquiti|access point|Ubiquiti long-range Wi-Fi 6 access point|U6-LR;U6 Long-Range|UniFi U6-Lite;UniFi U6-Pro;UniFi U6+
product|FortiGate 60F|fortinet|firewall|Fortinet small-business firewall|FG-60F;FGT60F|FortiGate 60E;FortiGate 80F;FortiWiFi 60F
product|Synology DS920+|synology|NAS|four-bay Synology network storage|DS920+;DS920 Plus|Synology DS923+;Synology DS220+;Synology DS1520+
product|PowerEdge R740|dell|server|Dell 2U rack server|R740;Dell R740|PowerEdge R750;PowerEdge R640;PowerEdge R740xd
product|EX4300|juniper networks|switch|Juniper access switch|Juniper EX4300|EX4400;EX3400;EX4300-MP
product|Surface Pro 9|microsoft|tablet|Microsoft 2-in-1 tablet|Microsoft Surface Pro 9|Surface Pro 8;Surface Laptop 5;Surface Pro X
product|ThinkPad X1 Carbon|lenovo|laptop|Lenovo business ultrabook|X1 Carbon|ThinkPad X1 Yoga;ThinkPad T14;ThinkPad X13
product|iPhone 15|apple|phone|Apple smartphone|Apple iPhone 15|iPhone 15 Pro;iPhone 14;iPhone 15 Plus
product|Pixel 8|google|phone|Google Android phone|Google Pixel 8|Pixel 8 Pro;Pixel 7;Pixel 8a
service|billing-api||service|internal billing and invoicing API|billing service|billing-worker;billing-ui;invoice-api
service|checkout-web||service|customer checkout web frontend|checkout frontend;checkout site|checkout-api;cart-web;checkout-worker
service|auth-service||service|login and token issuing service|auth svc;authentication service|authz-service;auth-proxy;oauth-service
service|search-indexer||service|job that feeds the search index|indexer|search-api;search-ui;search-crawler
service|notification-worker||service|sends email and push notifications|notifier;notifications worker|notification-api;newsletter-worker;alert-worker
service|inventory-api||service|stock levels API|inventory service;stock api|inventory-sync;inventory-ui;invoice-service
service|reporting-batch||service|nightly reporting jobs|reports batch;reporting jobs|reporting-api;reporting-ui;recording-batch
service|edge-proxy||service|public edge reverse proxy|edge reverse proxy|edge-cache;edge-router;egress-proxy
error|OOMKilled||error|container killed for exceeding its memory limit|out of memory kill;OOM kill|OOM score adjustment;out of disk;CrashLoopBackOff
error|ImagePullBackOff||error|Kubernetes cannot pull a container image|image pull backoff;ErrImagePull|ImagePullSecret;BackoffLimitExceeded;ContainerCreating
error|deadlock detected||error|database deadlock error|DB deadlock;deadlock|lock wait timeout;livelock;deadline exceeded
error|connection refused||error|TCP connection refused error|ECONNREFUSED;conn refused|connection reset;connection timed out;permission denied
error|segmentation fault||error|invalid memory access crash|segfault;SIGSEGV|bus error;stack overflow;SIGABRT
concept|rate limiting||concept|limiting how many requests a client may send|rate limit;throttling|load shedding;retry budget;rate card
concept|blue-green deployment||concept|switching traffic between two identical environments|blue/green deploy;blue green release|canary deployment;rolling update;shadow deployment
concept|circuit breaker||concept|pattern that stops calls to a failing dependency|breaker pattern|bulkhead;retry with backoff;circuit board
concept|feature flag||concept|runtime switch that turns code paths on or off|feature toggle;feature switch|feature branch;release train;feature request
concept|dead letter queue||concept|queue holding messages that could not be processed|DLQ|retry queue;priority queue;delay queue
concept|idempotency key||concept|client token that makes retried requests safe|idempotency token|API key;session key;correlation ID
"""

# Same-type names that are not stored at all: a merge with anything is wrong.
OUTSIDERS = {
    "software": "CockroachDB;Neo4j;InfluxDB;TimescaleDB;Couchbase;DynamoDB;Apache Pulsar;NATS;ZeroMQ;Caddy;Lighttpd;Podman;"
                "Nomad;Chef;Puppet;SaltStack;Kibana;Jaeger;Zipkin;Fluent Bit;Svelte;Ember.js;Laravel;Nest.js;Next.js;"
                "Remix;esbuild;Rollup;Parcel;Babel;Yarn;pnpm;Gradle;Maven;Bazel;CMake;Redpanda;KeyDB;Valkey;Dragonfly",
    "language": "Haskell;OCaml;Clojure;Lua;Perl;Julia;Zig;Nim;Dart;Fortran;COBOL;Erlang/OTP;Groovy;Visual Basic",
    "os": "Fedora 39;openSUSE Leap 15;Linux Mint 21;Manjaro;Pop!_OS 22.04;ChromeOS;HarmonyOS;Ubuntu 20.04;Windows 10;Debian 11",
    "vendor": "Aruba Networks;Palo Alto Networks;MikroTik;Linksys;ASUS;Huawei;ZTE;Zyxel;Oracle;IBM;VMware;Red Hat;Apple;Samsung",
    "product": "Linksys EA8300;ASUS RT-AX88U;MikroTik hEX S;Zyxel USG Flex 100;Aruba AP-515;Palo Alto PA-220;QNAP TS-453D;"
               "MacBook Air M2;Galaxy S23;HP EliteBook 840;Archer AX55;Nighthawk RAX120;Catalyst 3850;UniFi Switch 24",
    "service": "payments-api;shipping-service;user-profile;recommendation-engine;pricing-api;fraud-detector;email-gateway;audit-log",
    "error": "disk quota exceeded;too many open files;broken pipe;certificate expired;502 Bad Gateway;504 Gateway Timeout",
    "concept": "event sourcing;CQRS;sharding;backpressure;consistent hashing;saga pattern",
}


def key(s):
    """Surface identity for labelling: case, spaces, hyphens, underscores and dots ignored, symbols kept (C# != C++)."""
    return re.sub(r"[\s\-_.]", "", s.lower())


def typo(name, rng):
    tokens = [t for t in re.findall(r"[A-Za-z]+", name) if len(t) >= 5]
    if not tokens:
        return None
    t = max(tokens, key=len)
    i = rng.randrange(1, len(t) - 2)
    if t[i] == t[i + 1]:
        return None
    swapped = t[:i] + t[i + 1] + t[i] + t[i + 2:]
    return name.replace(t, swapped, 1)


def separators(name):
    out = []
    if " " in name:
        out.append(("separator", name.replace(" ", "-")))
        out.append(("joined", name.replace(" ", "")))
    if "-" in name:
        out.append(("separator", name.replace("-", " ")))
        out.append(("joined", name.replace("-", "")))
    return out


def other_version(name):
    groups = list(re.finditer(r"\d+", name))
    if not groups:
        return None
    g = groups[-1]
    n = int(g.group()) + 1
    return name[:g.start()] + str(n) + name[g.end():]


def entity_pairs():
    rng = random.Random(51)
    entities = []
    for line in CATALOGUE.strip().splitlines():
        typ, name, vendor, noun, desc, aliases, siblings = line.split("|")
        entities.append({
            "id": f"E{len(entities) + 1:03}",
            "type": typ,
            "name": name,
            "scope": {"vendor": vendor} if vendor else {},
            "noun": noun,
            "description": desc,
            "aliases": [a for a in aliases.split(";") if a],
            "siblings": [s for s in siblings.split(";") if s],
        })
    # Surface keys (per type) of everything stored: a negative mention must not literally be one of them.
    stored = {}
    for e in entities:
        for n in [e["name"]] + e["aliases"]:
            k = (e["type"], key(n))
            assert k not in stored or stored[k] == e["id"], (n, e["id"], stored.get(k))
            stored[k] = e["id"]

    pairs = []

    def pair(mention, typ, gold, kind, scope=None, of=None):
        # A variant that turned into another stored name of the same type is labelled with that entity (a conflicting
        # identity scope keeps it apart on purpose).
        k = (typ, key(mention))
        if k in stored and stored[k] != gold and kind != "conflicting-scope":
            gold = stored[k]
            kind = kind + "->stored"
        pairs.append({"mention": mention, "type": typ, "scope": scope or {}, "gold": gold, "kind": kind,
                      "of": of or gold})

    for e in entities:
        n, t, gid = e["name"], e["type"], e["id"]
        variants = [("exact", n), ("lower", n.lower())]
        if len(n) <= 12 and n.upper() != n:
            variants.append(("upper", n.upper()))
        variants += separators(n)
        variants.append(("suffix", f"{n} {e['noun']}"))
        vendor_display = {"tp-link": "TP-Link", "netgear": "Netgear", "cisco": "Cisco", "ubiquiti": "Ubiquiti",
                          "fortinet": "Fortinet", "synology": "Synology", "dell": "Dell", "juniper networks": "Juniper",
                          "microsoft": "Microsoft", "lenovo": "Lenovo", "apple": "Apple", "google": "Google"}
        if e["scope"]:
            v = vendor_display[e["scope"]["vendor"]]
            if not n.lower().startswith(v.lower()):
                variants.append(("vendor-prefix", f"{v} {n}"))
        else:
            variants.append(("article", f"the {n}"))
        ty = typo(n, rng)
        if ty:
            variants.append(("typo", ty))
        variants += [("alias", a) for a in e["aliases"]]
        seen = set()
        for kind, m in variants:
            if m in seen or (kind == "lower" and m == n):
                continue
            seen.add(m)
            pair(m, t, gid, kind)
        # Negatives.
        for s in e["siblings"]:
            assert (t, key(s)) not in stored, (s, e["id"])
            pair(s, t, None, "sibling", of=gid)
            extra = [("sibling-suffix", f"{s} {e['noun']}")]
            st = typo(s, rng)
            if st:
                extra.append(("sibling-typo", st))
            for kind, m in extra:
                pair(m, t, None, kind, of=gid)
        ov = other_version(n)
        if ov and (t, key(ov)) not in stored:
            pair(ov, t, None, "other-version", of=gid)
        if e["scope"]:
            other = "netgear" if e["scope"]["vendor"] != "netgear" else "tp-link"
            pair(n, t, None, "conflicting-scope", {"vendor": other}, of=gid)
    # Other types: the same name typed differently is another entity.
    other_type = {"software": "concept", "language": "software", "os": "product", "vendor": "product",
                  "product": "software", "service": "software", "error": "concept", "concept": "error"}
    for e in entities[::4]:
        pair(e["name"], other_type[e["type"]], None, "other-type", of=e["id"])
    for typ, names in OUTSIDERS.items():
        for n in names.split(";"):
            assert (typ, key(n)) not in stored, n
            pair(n, typ, None, "outsider")
            pair(f"{n} {rng.choice(['service', 'setup', 'instance', 'issue'])}" if typ != "error" else n.upper(),
                 typ, None, "outsider-variant")
    # Dedupe identical (mention, type, scope) keeping the first label.
    out, seen = [], set()
    for p in pairs:
        k = (p["mention"], p["type"], json.dumps(p["scope"], sort_keys=True))
        if k not in seen:
            seen.add(k)
            out.append(p)
    for i, p in enumerate(out):
        p["id"] = f"P{i + 1:04}"
    stored_entities = [{k: e[k] for k in ("id", "type", "name", "scope", "description", "aliases")} for e in entities]
    return {
        "about": "Entities as stored in memory and mentions labelled with the entity they name (gold) or null. "
                 "Generated by make_datasets.py; evaluated by entity_resolution.rs.",
        "entities": stored_entities,
        "pairs": out,
    }


def main():
    qa = graph_qa()
    (HERE / "graph_qa.json").write_text(json.dumps(qa, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    er = entity_pairs()
    (HERE / "entity_pairs.json").write_text(json.dumps(er, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    by = {}
    for q in qa["questions"]:
        by[q["type"]] = by.get(q["type"], 0) + 1
    pos = sum(1 for p in er["pairs"] if p["gold"])
    print(f"graph_qa.json: {qa['facts']} facts in {len(qa['episodes'])} episodes, {len(qa['questions'])} questions {by}")
    print(f"entity_pairs.json: {len(er['entities'])} entities, {len(er['pairs'])} pairs ({pos} same, "
          f"{len(er['pairs']) - pos} different)")


if __name__ == "__main__":
    main()
