"""entity_pairs.json: the entities as stored in memory and the mentions labelled with the entity each one names (see
make_datasets.py)."""

import json
import random
import re

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
