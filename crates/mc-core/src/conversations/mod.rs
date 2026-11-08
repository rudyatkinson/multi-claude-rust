//! Past conversations, read from the files Claude Code keeps them in (`src/main/conversations.ts`
//! and `src/shared/conversations.ts`): `<config dir>/projects/<folder key>/<conversation id>.jsonl`,
//! one JSON record per line. The files can be tens of megabytes, so only their beginning (the
//! first message, the folder) and their end (the latest title and last message) are read.
//!
//! - [`clean_prompt`]: a message as the user would recognize it;
//! - [`parse_head`] / [`parse_tail`]: what the two ends of a file say;
//! - [`scan_profile`] / [`ConversationIndex`]: the conversations of an account, read again only
//!   when a file changed;
//! - [`search_conversations`]: the search of the history dialog.

mod files;
mod json;
mod parse;
mod prompt;
mod search;

pub use files::{
    entry_of, read_head, read_tail, scan_profile, ConversationIndex, ScanCache, HEAD_BYTES, HEAD_MAX_BYTES, TAIL_BYTES, TITLE_CHARS,
};
pub use parse::{parse_head, parse_tail, Head, Tail, AI_TITLE_CHARS};
pub use prompt::{clean_prompt, PROMPT_CHARS};
pub use search::{fold_text, search_conversations};

pub(crate) use files::is_uuid;
