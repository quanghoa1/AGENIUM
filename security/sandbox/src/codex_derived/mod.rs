//! Code adapted from OpenAI Codex (Apache License 2.0). Everything in this module, and nothing outside it, is
//! derived from the vendored upstream sources in `vendor/codex-sandbox/upstream/` (see `vendor/codex-sandbox/VENDOR.md`
//! for the file-to-file map). When `devtools/scripts/sync-codex-sandbox.sh <ref> --summary` reports a changed upstream file,
//! update the matching file here; Agenium-specific behaviour belongs in the parent modules.

#[cfg(windows)]
pub mod mxc_policy;
#[cfg(windows)]
pub mod mxc_probe;
