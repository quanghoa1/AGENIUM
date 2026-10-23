-- Agenium v1.0 — SurrealDB 3.x, namespace `agenium`, database `core`
-- Engine (SurrealKV | RocksDB) chốt theo spike S1. Phương án B: SQLite + sqlite-vec (cùng Repo trait).
-- Quy tắc: ghi event trước, cập nhật trạng thái sau, cùng transaction (quyết định 22).
DEFINE NAMESPACE IF NOT EXISTS agenium; USE NS agenium; DEFINE DATABASE IF NOT EXISTS core; USE DB core;

DEFINE TABLE meta SCHEMAFULL;
DEFINE FIELD schema_version ON meta TYPE int;

DEFINE TABLE project SCHEMAFULL;
DEFINE FIELD name ON project TYPE string;
DEFINE FIELD path ON project TYPE string;                -- folder có agenium.toml
DEFINE FIELD key_prefix ON project TYPE string DEFAULT "AGN";
DEFINE FIELD next_seq ON project TYPE int DEFAULT 1;
DEFINE FIELD status ON project TYPE string ASSERT $value IN ["Active","Completed","Cancelled"];
DEFINE FIELD git ON project TYPE object;                 -- { main_branch, lfs, remote: option<{ url, ssh_key_ref, last_test }> }
DEFINE FIELD allowlist ON project TYPE array<string> DEFAULT [];  -- chỉ người dùng thêm vĩnh viễn ("luôn cho phép")
DEFINE FIELD created_at ON project TYPE datetime DEFAULT time::now();
DEFINE INDEX project_path ON project FIELDS path UNIQUE;

DEFINE TABLE team SCHEMAFULL;
DEFINE FIELD name ON team TYPE string;
DEFINE FIELD template ON team TYPE string;               -- "coding@1"
DEFINE FIELD project ON team TYPE option<record<project>>;  -- một project tại một thời điểm
DEFINE FIELD lead ON team TYPE option<record<agent>>;
DEFINE FIELD solo ON team TYPE bool DEFAULT false;

DEFINE TABLE agent SCHEMAFULL;
DEFINE FIELD name ON agent TYPE string;
DEFINE FIELD team ON agent TYPE record<team>;            -- đúng một team
DEFINE FIELD role ON agent TYPE string;
DEFINE FIELD tags ON agent TYPE array<string> DEFAULT [];
DEFINE FIELD status ON agent TYPE string ASSERT $value IN ["Idle","Working","Waiting","QuotaExhausted","Offline"];
DEFINE FIELD backend ON agent TYPE string ASSERT $value IN ["Api","Cli"];
DEFINE FIELD model ON agent TYPE object;                 -- { provider, model, effort }
DEFINE FIELD memory_db ON agent TYPE string;             -- "agent_<ulid>"
DEFINE FIELD mcp ON agent TYPE array<object> DEFAULT [];  -- { server, credential_ref }
DEFINE FIELD skills ON agent TYPE array<string> DEFAULT [];

DEFINE TABLE spec SCHEMAFULL;
DEFINE FIELD project ON spec TYPE record<project>;
DEFINE FIELD version ON spec TYPE int;
DEFINE FIELD body ON spec FLEXIBLE TYPE object;          -- SCHEMAS/requirement-spec.schema.json
DEFINE FIELD approved_by ON spec TYPE option<string>;
DEFINE FIELD locked_at ON spec TYPE option<datetime>;
DEFINE INDEX spec_ver ON spec FIELDS project, version UNIQUE;

DEFINE TABLE plan SCHEMAFULL;
DEFINE FIELD project ON plan TYPE record<project>;
DEFINE FIELD version ON plan TYPE int;
DEFINE FIELD spec_version ON plan TYPE int;
DEFINE FIELD reason ON plan TYPE option<string>;
DEFINE FIELD diff ON plan TYPE option<string>;
DEFINE FIELD body ON plan FLEXIBLE TYPE object;          -- milestones + items (plan.schema.json)
DEFINE FIELD approved_by ON plan TYPE option<string>;
DEFINE INDEX plan_ver ON plan FIELDS project, version UNIQUE;

DEFINE TABLE ticket SCHEMAFULL;
DEFINE FIELD key ON ticket TYPE string;                  -- AGN-n
DEFINE FIELD project ON ticket TYPE record<project>;
DEFINE FIELD parent ON ticket TYPE option<record<ticket>>;
DEFINE FIELD kind ON ticket TYPE string ASSERT $value IN ["Epic","Story","Task","Bug","Requirement"];
DEFINE FIELD title ON ticket TYPE string;
DEFINE FIELD description ON ticket TYPE string;
DEFINE FIELD acceptance ON ticket TYPE array<object> ASSERT array::len($value) > 0;
DEFINE FIELD status ON ticket TYPE string ASSERT $value IN ["New","Assigned","InProgress","Pending","Resolved","Reopened","Closed","Cancelled"];
DEFINE FIELD assignee ON ticket TYPE option<record<agent>>;
DEFINE FIELD plan_item ON ticket TYPE string;            -- bắt buộc
DEFINE FIELD priority ON ticket TYPE int ASSERT $value >= 0 AND $value <= 3;
DEFINE FIELD depends_on ON ticket TYPE array<record<ticket>> DEFAULT [];  -- DAG, kiểm tra chu trình ở orchestrator
DEFINE FIELD required_role ON ticket TYPE option<string>;
DEFINE FIELD required_tags ON ticket TYPE array<string> DEFAULT [];
DEFINE FIELD scope_paths ON ticket TYPE array<string> DEFAULT [];
DEFINE FIELD report_template ON ticket TYPE string;
DEFINE FIELD approved ON ticket TYPE bool DEFAULT false; -- review_ticket approve
DEFINE FIELD merged ON ticket TYPE bool DEFAULT false;   -- merge queue thành công (ticket code)
DEFINE FIELD has_code ON ticket TYPE bool DEFAULT false;
DEFINE FIELD reopen_count ON ticket TYPE int DEFAULT 0;
DEFINE FIELD escalated ON ticket TYPE bool DEFAULT false;
DEFINE FIELD close_reason ON ticket TYPE option<string>;
DEFINE FIELD usage ON ticket FLEXIBLE TYPE object DEFAULT {}; -- tokens + estimated_cost_usd, chỉ hiển thị
DEFINE FIELD updated_at ON ticket TYPE datetime VALUE time::now();
DEFINE INDEX ticket_key ON ticket FIELDS project, key UNIQUE;
DEFINE INDEX ticket_status ON ticket FIELDS project, status;
DEFINE INDEX ticket_assignee ON ticket FIELDS assignee, status;

-- Ticket sẵn sàng (scheduler): mọi dependency Closed, hoặc Resolved + approved (+ merged nếu has_code)
-- SELECT * FROM ticket WHERE status IN ["Assigned","Reopened"] AND array::all(depends_on.map(|$d|
--   $d.status = "Closed" OR ($d.status = "Resolved" AND $d.approved AND (!$d.has_code OR $d.merged))));

DEFINE TABLE ticket_workspace SCHEMAFULL;
DEFINE FIELD ticket ON ticket_workspace TYPE record<ticket>;
DEFINE FIELD worktree ON ticket_workspace TYPE string;
DEFINE FIELD branch ON ticket_workspace TYPE string;     -- agenium/<ticket-id>
DEFINE FIELD base_sha ON ticket_workspace TYPE string;
DEFINE FIELD head_sha ON ticket_workspace TYPE option<string>;
DEFINE FIELD merged_sha ON ticket_workspace TYPE option<string>;
DEFINE INDEX tw_ticket ON ticket_workspace FIELDS ticket UNIQUE;

DEFINE TABLE ticket_link TYPE RELATION IN ticket OUT ticket SCHEMAFULL;
DEFINE FIELD kind ON ticket_link TYPE string ASSERT $value IN ["related_to","blocks","duplicates","follow_up_of","aggregates"];
DEFINE FIELD confirmed ON ticket_link TYPE bool DEFAULT false;

DEFINE TABLE comment SCHEMAFULL;
DEFINE FIELD ticket ON comment TYPE record<ticket>;
DEFINE FIELD kind ON comment TYPE string ASSERT $value IN ["Question","Answer","Progress","Report","Review","Permission","Handoff","System","User"];
DEFINE FIELD author ON comment TYPE string;              -- agent:<id> | user | system
DEFINE FIELD reply_to ON comment TYPE option<record<comment>>;
DEFINE FIELD body ON comment FLEXIBLE TYPE object;       -- form có cấu trúc theo kind
DEFINE FIELD blocking ON comment TYPE bool DEFAULT false;
DEFINE FIELD resolved ON comment TYPE bool DEFAULT false;
DEFINE FIELD created_at ON comment TYPE datetime DEFAULT time::now();
DEFINE INDEX comment_ticket ON comment FIELDS ticket, created_at;

DEFINE TABLE permission_grant SCHEMAFULL;
DEFINE FIELD ticket ON permission_grant TYPE record<ticket>;
DEFINE FIELD run ON permission_grant TYPE option<string>; -- grant của Lead: luôn có run (theo run)
DEFINE FIELD scope ON permission_grant TYPE string ASSERT $value IN ["run","project"];
DEFINE FIELD granted_by ON permission_grant TYPE string;
DEFINE FIELD request ON permission_grant FLEXIBLE TYPE object;
DEFINE EVENT lead_only_run ON permission_grant WHEN $event = "CREATE" AND $after.scope = "project" AND $after.granted_by != "user"
  THEN { THROW "always-allow (scope=project) chỉ người dùng cấp được"; };

DEFINE TABLE run SCHEMAFULL;
DEFINE FIELD ticket ON run TYPE option<record<ticket>>;
DEFINE FIELD agent ON run TYPE record<agent>;
DEFINE FIELD started_at ON run TYPE datetime;
DEFINE FIELD ended_at ON run TYPE option<datetime>;
DEFINE FIELD outcome ON run TYPE option<string>;          -- Completed | Stopped(reason) | Error
DEFINE FIELD steps ON run TYPE int DEFAULT 0;
DEFINE FIELD usage ON run FLEXIBLE TYPE object DEFAULT {}; -- tokens_in/out, cache_read/write, estimated_cost_usd
DEFINE FIELD provider ON run TYPE string;

DEFINE TABLE event SCHEMAFULL;                            -- append-only, nguồn sự thật cho resync UI
DEFINE FIELD seq ON event TYPE int;
DEFINE FIELD intent_id ON event TYPE option<string>;      -- idempotency
DEFINE FIELD kind ON event TYPE string;
DEFINE FIELD ticket ON event TYPE option<record<ticket>>;
DEFINE FIELD actor ON event TYPE string;
DEFINE FIELD payload ON event FLEXIBLE TYPE object;
DEFINE FIELD at ON event TYPE datetime DEFAULT time::now();
DEFINE INDEX event_seq ON event FIELDS seq UNIQUE;
DEFINE INDEX event_intent ON event FIELDS intent_id UNIQUE;

DEFINE TABLE model_cache SCHEMAFULL;
DEFINE FIELD provider ON model_cache TYPE string;
DEFINE FIELD models ON model_cache FLEXIBLE TYPE array;
DEFINE FIELD fetched_at ON model_cache TYPE datetime;
