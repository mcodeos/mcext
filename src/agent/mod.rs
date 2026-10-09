//! In-host LLM agent loop (P6·S1 pilot bed).
//!
//! Four pieces, each independently portable (the whole module moves to the
//! bench host later; it must not import editor-facing types):
//!
//! - [`config`]: endpoint/model/limits — user machine state, never in the
//!   repo and never in a contract face;
//! - [`mapping`]: caps → LLM function-calling tools. Consumes the caps JSON
//!   only; there is no second copy of the method semantics here;
//! - [`adapter`]: OpenAI-compatible chat-completions wire (protocol-only —
//!   any intranet endpoint speaking the protocol works);
//! - [`session`]: the turn loop (user message → model → tool calls → rpc
//!   dispatch → results → model → final text) behind a trait-seamed rpc
//!   dispatch so tests can run without a daemon.

pub mod adapter;
pub mod config;
pub mod mapping;
pub mod session;
