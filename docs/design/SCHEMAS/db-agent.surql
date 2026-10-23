-- Agenium v1.0 — database riêng cho mỗi agent: `agent_<ulid>` (memory riêng tư, quyết định 2)
-- Embedding 384 chiều (bge-small-en-v1.5); đổi model theo spike S6 thì REBUILD INDEX + re-embed.
USE NS agenium; DEFINE DATABASE IF NOT EXISTS agent_ULID; USE DB agent_ULID;

DEFINE TABLE fact SCHEMAFULL;
DEFINE FIELD text ON fact TYPE string;                    -- tiếng Anh
DEFINE FIELD embedding ON fact TYPE array<float>;
DEFINE FIELD kind ON fact TYPE string ASSERT $value IN ["fact","preference","lesson"];
DEFINE FIELD scope ON fact FLEXIBLE TYPE object DEFAULT {}; -- vendor/product/version/language/project/domain
DEFINE FIELD cue ON fact TYPE option<string>;
DEFINE FIELD evidence ON fact TYPE option<string>;
DEFINE FIELD source ON fact TYPE string;                  -- ticket:<key> | chat | review
DEFINE FIELD untrusted ON fact TYPE bool DEFAULT false;
DEFINE FIELD confidence ON fact TYPE float DEFAULT 0.5;
DEFINE FIELD weight ON fact TYPE float DEFAULT 1.0;       -- decay half-life 90 ngày, ẩn < 0.2
DEFINE FIELD superseded_by ON fact TYPE option<record<fact>>;
DEFINE FIELD created_at ON fact TYPE datetime DEFAULT time::now();
DEFINE FIELD used_at ON fact TYPE option<datetime>;
DEFINE INDEX fact_vec ON fact FIELDS embedding HNSW DIMENSION 384 DIST COSINE;
DEFINE ANALYZER en_simple TOKENIZERS class FILTERS lowercase, snowball(english);
DEFINE INDEX fact_bm25 ON fact FIELDS text FULLTEXT ANALYZER en_simple BM25;

DEFINE TABLE conversation SCHEMAFULL;                     -- chat Solo/Lead với người dùng
DEFINE FIELD started_at ON conversation TYPE datetime;
DEFINE FIELD closed_at ON conversation TYPE option<datetime>;
DEFINE FIELD messages ON conversation FLEXIBLE TYPE array;

DEFINE TABLE skill_overlay SCHEMAFULL;
DEFINE FIELD skill ON skill_overlay TYPE string;
DEFINE FIELD base_version ON skill_overlay TYPE string;
DEFINE FIELD patch ON skill_overlay TYPE string;
DEFINE FIELD changelog ON skill_overlay TYPE array<object> DEFAULT [];
