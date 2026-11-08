# Changelog

All notable changes to Multi Claude. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This file is shown inside the app under **What's new**.

## [0.6.0] - 2026-11-08

### Added
- A **What's new** button at the bottom of the sidebar opens this changelog. A dot on the button marks a version you have not read yet.
- **Tab overflow menu.** When an area has more tabs than fit in its strip, a `▾` button next to `+` lists them all; pick one to bring it into view.
- **Vertical tabs.** Settings → Window → *Tab strip* puts each area's tabs in a column on its left instead of a row on top.
- **Workflows and Library open as tabs.** They sit in an area like any session, can be dragged between areas and come back after a restart. The sidebar buttons bring the tab to the front; Escape or Ctrl+Shift+W closes it.
- The sidebar footer shows both versions: `Claude Code <version> | Multi Claude <version>`.

### Changed
- **Rewritten in Rust.** The backend that used to run on Electron and Node is now a Rust program on Tauri 2 with the system WebView. The installer is a fraction of its former size, the app starts faster and uses far less memory. Accounts, tabs, workflows and settings in `~/.multi-claude` are read as they are; nothing has to be migrated.
- Sidebar footer icons are larger (18 px in 30 px buttons); they were hard to hit, above all on macOS.
- Tabs are dragged with the pointer directly instead of the browser's drag-and-drop, which makes the drop preview follow the mouse without delay.
- OAuth tokens of token accounts are kept in the operating system's credential store (Windows Credential Manager, macOS Keychain). Tokens saved by the Electron version cannot be read by this one; such accounts ask for the token again once.

## [0.5.1] - 2026-10-19

### Fixed
- Desktop notifications for a session that stopped were not shown in some cases.
- macOS: the packaged app starts sessions, shows its menu bar icon and quits cleanly.

## [0.5.0] - 2026-10-08

### Added
- **Usage limits.** Every tab reports its turn through hooks the app adds to the session: the tab mark and the notification say whether Claude finished, asks for a permission, asks a question, presents a plan, stopped at a usage limit or ended with an error.
- A bar under a session that ran into its limit offers **Continue on …** with the account that has the most room left. The conversation is copied over and Claude picks the task up where it stopped.
- Settings → Accounts → *Move a conversation to another account by itself* does that without asking, within strict rules (two things must confirm the limit, at most two moves in five hours, the target needs room).
- The app reads Claude Code's own wait at a usage limit and tells you how it ended.
- `~/.multi-claude/limit-events.json` keeps the last twenty failed turns.

## [0.4.0] - 2026-10-08

### Added
- **Library** panel with four sections: Skills, Agents, Memory and Instructions, all edited as text with conflict detection when a file changed on disk.
- **Agents:** subagent files per account or project, with a form for their frontmatter.
- **Memory:** Claude Code's auto memory per account and project, with search, warnings about a long or stale `MEMORY.md`, and sharing the main account's memory with other accounts (`autoMemoryDirectory`). **Bring into main's memory** merges an account's own notes.
- **Instructions:** `CLAUDE.md`, `CLAUDE.local.md` and rules per account and project, imports listed, and a view of what a session reads and in which order.

## [0.3.0] - 2026-10-08

### Added
- **Skills** manager: personal and project skills, a form and a source view of `SKILL.md` that edit the same text, checks Claude Code and claude.ai would stumble over, rename, duplicate, copy or move between folders, export and import as `.zip` / `.skill`, and **Ask Claude…** to hand a request about a skill to a session.

## [0.2.0] - 2026-10-07

### Added
- **Workflows:** chain sessions across accounts. Each step has its own account, model, effort and permission mode, continues a new or an earlier conversation and can wait for approval. Prompts use `{{task}}`, `{{folder}}`, `{{previous.output}}` and `{{steps.Name.output}}`. A bar above the terminals shows each run with Continue, Retry, Use last answer and Stop.

## [0.1.0] - 2026-10-07

### Added
- Accounts with separate Claude Code profile folders, login in the browser or by OAuth token, shared `agents`, `skills`, `commands` and `plugins` folders, copied `settings.json`, `CLAUDE.md` and `keybindings.json` with a sync indicator, and **Copy MCP servers from Main**.
- Terminal tabs with Claude Code's topic as title, arranged in areas that split, resize and merge; drag and drop of tabs, files and folders; search in the terminal; a list of past conversations of every account.
- Usage per account from `/usage` and the live status line, with alerts at 80% and 95%.
- Tray icon, start with Windows, **Open in Multi Claude** in the File Explorer menu, and per-account launchers `claude-<name>` for any terminal.
