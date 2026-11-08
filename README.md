# Multi Claude

A desktop app for using Claude Code with several Claude accounts side by side. Each account logs in separately in its own profile folder, and sessions run in terminal tabs inside the app.

The app is built on Tauri 2: a Rust backend, and a window made with React and xterm.js that runs in the system's WebView (WebView2 on Windows, WKWebView on macOS). It replaces the Electron version and uses the same data folder, `~/.multi-claude`: accounts, tabs, workflows and settings are read as they are, and nothing has to be migrated. The one exception is the OAuth token of a token account. It is now kept in the operating system's credential store, and this version cannot read the token the Electron version saved, so such an account asks for its token once (see [Token accounts](#token-accounts)).

## Running

```
npm install
npm run tauri dev      # open in development mode
npm run tauri build    # installer: target/release/bundle/nsis/Multi Claude_<version>_x64-setup.exe
                       # on a Mac: target/release/bundle/dmg/Multi Claude_<version>_<arch>.dmg
```

The Cargo workspace keeps `target` at the top of the repository, not in `src-tauri`. `npm run dist` is the same build; on a Mac, add `-- --target x86_64-apple-darwin` for Intel. The build also signs the updater's files and stops without the signing key: see [Building locally](#building-locally).

Other commands: `npm test` (the window's tests), `cargo test --workspace` or `npm run rust:test` (the backend's tests), `npm run typecheck`, `npm run build` (only the window, into `dist/`), `npm run build:shell` (the context menu DLL), `npx tauri icon src-tauri/icons/source.png` (the app icons; the tray icons `tray.png` and `tray@2x.png` are separate).

Building needs Node.js 22.12 or newer (CI uses 24) and Rust stable (1.85 or newer); on Windows also the MSVC toolchain with Visual Studio Build Tools, on macOS the Xcode Command Line Tools.

Using it needs Claude Code (`claude` is found automatically; otherwise set its path in Settings) and, on Windows, WebView2: Windows 11 has it, and on Windows 10 the installer downloads it when it is missing. Live usage updates from running sessions need Git Bash on Windows.

To run a second instance next to the installed app, for example while developing, set `MULTI_CLAUDE_HOME` to an empty folder. That instance keeps all of its data there instead of `~/.multi-claude`, has its own single-instance lock and WebView data, and shares nothing with the installed app. Without it, a development run uses the installed app's data folder and lock.

## Accounts

- The **main** profile is the existing `~/.claude` folder, used as it is.
- **Add account** creates a new profile folder (`~/.multi-claude/profiles/<name>`) and opens a login tab. The login happens in the browser; if another Claude account is signed in there, open the link in a private window.
- The main profile's `agents`, `skills`, `commands` and `plugins` folders are linked into the new profile (junctions on Windows). `settings.json`, `CLAUDE.md` and `keybindings.json` are copied; when they change in the main profile, the card shows a sync icon.
- With "Keep model and effort settings separate" ticked, a sync leaves the profile's own `model`, `effortLevel` and `modelSettings` alone.
- MCP servers, the theme and folder trust stay separate for each account. Install plugins from the main account.
- **Copy MCP servers from Main…** in an account's menu copies the main account's user-level MCP servers into that account, replacing servers of the same name. It goes through `claude mcp add-json`, so it needs the native Claude Code install (`claude.exe`), and the app never shows or stores the definitions. Servers that sign in through the browser need `/mcp` again in each account; servers of a single project are not copied; open sessions see the copied servers after a restart.
- Deleting an account moves its profile folder to the Recycle Bin; the main profile's files are not touched.

### Token accounts

**Add account → OAuth token** adds an account from a token created with `claude setup-token` instead of a browser login. Its sessions run with `CLAUDE_CODE_OAUTH_TOKEN`. The token is kept in the operating system's credential store (Windows Credential Manager, macOS Keychain) under the name `multi-claude`; `config.json` only notes that it is there, and the backend never hands it to the window.

A token account added by the Electron version shows "Token unavailable" on its card: that version kept the token encrypted in `config.json`, in a form only Electron can read. **Replace token** on the card, or in the account's menu, takes it once more.

Claude Code reports less for these accounts, so they are more limited:

- No usage numbers, email or plan on the card.
- The token is not checked up front; an invalid one shows as an error when a session starts.
- No external terminal and no `claude-<name>` launcher, since both would need the token in a plain text file.

## Sidebar

The buttons at the foot of the sidebar collapse or expand it (Ctrl+Shift+B), open Workflows and the Library, find a past conversation (Ctrl+Shift+H), and open **What's new** and Settings. **What's new** shows the changelog; a dot on it marks a version you have not read yet. Under the buttons stand both versions, `Claude Code <version> | Multi Claude <version>`, followed by `↑ <version>` while an update waits (see [Updates](#updates)).

## Tabs

A tab shows the conversation's topic as Claude Code names it, with the folder, the account and the model underneath. Until a conversation has a topic, the folder name is the title. The mark on the left is a spinning ring while Claude is working, a dot while it waits for your input, a grey crescent while the tab sleeps (see [Sleeping tabs](#sleeping-tabs)), and a hollow ring once the session has ended. Three more marks say why Claude stopped: an amber square when it asks for a permission, asks a question or presents a plan, a red diamond when its turn ended with an API error, and a hollow amber square when a usage limit cut the turn off. Hover the mark to read which it is. The number on the right is how full the conversation's context window is; it turns amber at 60% and red at 85%.

The app knows why Claude stopped through hooks it adds to every session it starts. They post to a port on this computer only and come on top of your own hooks. If hooks are switched off in Claude Code's settings (`disableAllHooks`), or the app cannot open its port, tabs work without them: the mark then only tells working from waiting, as read from the terminal title.

The **+** in the tab bar picks the account and the folder of a new session. Each account shows how much of its 5-hour limit is used (its weekly limit, for a plan without a 5-hour limit). A session starts in one of three ways:

- **New:** a new conversation.
- **Continue last:** the folder's most recent conversation (`claude -c`).
- **From history:** Claude Code's own list of the folder's earlier conversations (`claude --resume`); the one you pick continues in the tab.

### Arranging tabs

- **Drag** a tab along its strip to reorder it, or into another area (see below). The order is kept when the app restarts.
- **Double-click** a tab, or right-click it and choose **Rename…**, to give it a name of your own. The name replaces the conversation's topic until you clear it, and it stays with the tab across restarts.
- **Right-click** a tab for more: **Duplicate** opens a new conversation with the same account and folder next to it, **Open folder in Explorer** shows its folder, and **Close others** and **Close tabs to the right** close several tabs of the same area after a single confirmation.
- **Middle-click** closes a tab.
- When an area has more tabs than its strip can show, a `▾` with their number appears next to its **+**. It lists every tab of the area (the arrow keys, Home and End move through the list); pick one to bring it into view.
- Settings → **Window → Tab strip: Left** puts each area's tabs in a column on its left instead of a row on top, with **+** and `▾` at its foot.

### Areas

The window can be divided into areas, each with its own tab strip and its own **+**. Drag a tab to make or change them:

- onto the **left, right, top or bottom part of an area**: that area is split and the tab gets the new half;
- onto the **middle of an area** or onto its **tab strip**: the tab moves into that area;
- onto the **left, right or bottom edge of the window** (when there are several areas): the tab gets an area along that whole side.

A highlight shows what a drop would do. An area whose last tab leaves or closes disappears, and its neighbour takes the room. Drag the line between two areas to resize them (double-click for equal halves); an area cannot be made smaller than it can be used.

Without the mouse: Ctrl+Shift+D gives the active tab an area of its own on the right, and a tab's right-click menu has **Split right**, **Split down** and **Merge all areas**. F6 and Shift+F6 move the keyboard from area to area; clicking into an area does the same. The area that has the keyboard shows its tab with a full line in the account's colour, the others with a faint one.

The arrangement is saved with the tabs and comes back when the app restarts.

### Workflows and Library tabs

Workflows and the Library each open in a tab of their own, in the area that has the keyboard. Their sidebar button brings the tab to the front, or closes it when it is in front already. Escape (while the keyboard is in it) and Ctrl+Shift+W close it too, without asking: what is typed in the Library stays until it is saved or the app quits. Otherwise it behaves like a session's tab: it can be dragged and split off, Ctrl+Tab reaches it, and it comes back after a restart.

### Sleeping tabs

Every tab runs its own `claude` process, a couple of hundred megabytes each, even when nobody has touched it for an hour. A tab that has been idle for a while therefore goes to sleep: its process ends, but the tab keeps its place, its name, what its terminal last showed and its conversation. The terminal stays on screen, dimmed, with **Sleeping · Wake** in the middle; the mark is a grey crescent, and hovering it says since when the tab sleeps.

- **Waking.** A click anywhere in the terminal, or any key while the keyboard is in it, wakes the tab: Claude starts again in the same tab with its conversation (`claude --resume`), or the folder's last one when the conversation was never written, and the keyboard goes to it. The key that woke it is not typed. Bringing the tab to the front (a click on its tab, Ctrl+Tab) does not wake it, so you can look at what it last showed.
- **When a tab sleeps.** Settings → **Sessions → Put idle sessions to sleep after … minutes** sets how long a tab has to be idle (15 minutes unless you change it; 0 never). Idle means nothing was typed into it and, once Claude finished its answer, nothing happened since; a tab without hooks also counts what Claude Code prints. A tab never sleeps while Claude works or waits for work it started in the background, while it asks for a permission, asks a question, presents a plan, stopped at a usage limit or with an error, while a workflow run waits for it, while it is being moved to another account, or while you look at it (the tab in front in the area that has the keyboard, with the window in front). Login tabs never sleep.
- **By hand.** A tab's right-click menu has **Sleep now** and **Wake**, and **Keep awake**, which keeps that tab from ever going to sleep by itself. Ctrl+Shift+Z puts the active tab to sleep or wakes it.
- A sleeping tab counts as not running: closing it does not ask, the tray and the account cards do not count it, and quitting does not ask because of it. It sends no notifications and shows no limit bar.
- Tabs that are asleep when the app quits come back asleep the next time it starts, without starting anything. Quitting itself puts nothing to sleep: running tabs end and come back running, as before.

### Dropping files

Drop files or folders from Explorer onto a terminal to type their paths into it, quoted where needed, for example to hand Claude a screenshot or a file to read. Drop a folder onto an area's tab strip to open a session there in that folder; a file dropped there stands for the folder it is in.

### Finding text

Ctrl+Shift+F opens a search bar over the terminal that has the keyboard. Enter and Shift+Enter go to the next and the previous match, Escape closes it. The search covers what the terminal has kept, the last 10,000 lines; with Claude Code's fullscreen renderer (`/tui fullscreen`) there is no such history to search.

### Continuing on another account

Right-click a tab and choose **Continue on another account…** to carry its conversation over, for example when the account has run into its limit. The session on the old account ends, the conversation's files are copied into the other account's profile folder, and the same tab starts again there on the same conversation.

- The old account keeps its copy; it stays in that account's history.
- The first message on the new account sends the whole conversation again, since nothing of it is cached there. With a full context window that message is expensive.
- If Claude is still working when you move the conversation, its current answer is cut short.
- A folder the new account has not used yet asks for trust first, as any new folder does.
- Moving a conversation back replaces the copy it left behind. If that copy was continued separately in the meantime, the move is refused rather than overwriting it; open it from **From history** on that account instead.

#### At a usage limit

A bar under a session offers **Continue on …** with the logged-in account that has the most room left, and **Other account…** for a choice. It shows in two cases:

- A usage limit cut the session's last turn off, as its hooks report. Continuing then also asks Claude on the new account to pick the task up where it stopped, and a line under the tab says where the conversation came from and why. With the npm `.cmd` shim that message cannot be sent; send one yourself.
- An account's 5-hour or weekly limit, or the weekly limit of the model the session runs on, stands at 100%. This is a suggestion: an account with extra usage enabled keeps working at 100%.

The bar says which of the two it knows from, and stays away while Claude is answering. Its × hides it until that limit starts over.

Claude Code itself (2.1.234 and later) waits in the open session when a usage limit stops it, and goes on with the task after the reset. The app leaves that to Claude Code and only tells you how the wait ends when it does not simply go on: the limit reset while the computer slept and the tab wants Enter, or Claude Code gave the wait up.

#### Moving by itself

Settings → **Accounts → Move a conversation to another account by itself when a usage limit cuts it off** does the above without you: the conversation moves to the logged-in account with the most room and Claude picks the task up there. It is off unless you switch it on, because the first message on the new account sends the whole conversation again.

- Two things have to say that the limit is reached: the session's hooks, and usage numbers the app fetches at that moment. A rate limit error alone can be the server slowing everyone down; 100% alone can be an account that goes on with extra usage. Without both, nothing moves and the bar is all you get.
- The account it moves to is asked for its numbers as well and needs room: at most 80% of its 5-hour limit and less than 95% of its weekly limits used.
- A conversation is moved by itself at most twice in five hours, not within ten minutes of its last move, and not back to an account it came from. An account takes one conversation at a time: when several tabs run into the limit together, the first moves and the others keep the bar.
- Token accounts report no usage, so a conversation is never moved from or to one by itself. Workflow steps are not moved either; the run pauses as before.
- When no account has room the tab stays, the app says so, and Claude Code's own wait for the reset goes on.
- A folder the new account has not used asks for trust first, which holds the message back. The app points that out after a moment.
- The setting needs the native `claude` executable.

### Finding a past conversation

The clock button at the foot of the sidebar (Ctrl+Shift+H) lists the conversations of every account, newest first. Type to narrow the list: every word has to occur in the title, the first or the last message, the folder or the account's name, and "isik" finds "Işık". Enter or a click opens the conversation in a tab with the account and the folder it belongs to; a conversation that is already open is brought to the front instead.

The list is read from Claude Code's own conversation files (their beginning and end only, so large conversations cost nothing) and kept in memory. It searches titles and first and last messages, not the whole text of the conversations.

### Notifications

When a session that worked for a few seconds stops and waits for your input while you are looking elsewhere, its tab title turns bold until you open it. If the window is not in front, the taskbar button flashes and a desktop notification names the conversation and the account; clicking it brings that tab to the front. Settings → **Sessions** switches the notification off.

The notification says what the session stopped for: it finished its answer, asks for a permission, asks a question, presents a plan, stopped at a usage limit, or ended with an error, which it names. A session that asks for something or fails calls for attention at once, however briefly it worked. Without hooks the notification says "is waiting for your input".

### Reopening tabs

The open tabs and the areas they are arranged in are saved as they change. The next time the app starts, each tab opens again in its area with the same account and folder and continues its conversation; a tab that never got a message starts a new one. A tab that was asleep comes back asleep and starts only when it is woken; one that was asleep without ever getting a message is left out. Tabs whose account or folder no longer exists, login tabs and ended sessions are left out too. Settings → **Sessions** switches this off.

## Workflows

A workflow runs sessions one after the other and hands each one's answer to the next. Every step has its own account, model and prompt: one account can plan with Fable, another implement with Opus, and the first review the result in the conversation in which the plan was made.

### Designing

The chain button at the foot of the sidebar opens the designer in its tab (see [Workflows and Library tabs](#workflows-and-library-tabs)); the sessions in the other tabs keep running. The steps stand in a row from left to right: click one to change it, drag it to move it, **+** adds one.

A step has

- an **account**, and if you want a **model**, an **effort** and a **permission mode** for its session. "Default" leaves each of them to the account.
- a **conversation**: a new one, or the conversation of an earlier step. A step that goes on with an earlier conversation knows everything that was said in it; a dashed line under the chain joins the two. If its account is another one, the conversation is copied there first, as with *Continue on another account*.
- a **prompt**. In it `{{task}}` stands for what you type when you start the run, `{{previous.output}}` for the answer of the step before, `{{steps.Name.output}}` for the answer of any earlier step and `{{folder}}` for the folder. Anything else in double braces is sent as it is written.
- a **gate**: with "Wait for my approval" the run stops after the step until you let it continue. New steps have it.

The designer marks a step that cannot run (no prompt, a deleted account, the output of a step that comes later) and says why. The first time it is opened it holds a sample: plan, implement, review.

### Running

**Run…** asks for the folder all steps work in, and for the task. Every step opens in a tab of its own, named after the step, next to the tab of the step before. A step that goes on with an earlier conversation takes over that conversation's tab. They are ordinary tabs: you can read along, answer Claude's questions and permission prompts, and type.

A bar above the terminals shows each run: its steps, the one it is at, and what it waits for.

- A step is finished when Claude ends its answer; its last message is the step's output, and ≡ on the step shows it. In plan mode a step is finished when Claude presents its plan, and the plan is the output.
- At a gate, read the result. You can go on talking to Claude in the tab, for example when it ended with a question; the latest answer becomes the output. **Continue to …** starts the next step.
- When Claude waits for you in a step's tab (a permission, a question, the trust question of a folder the account has not used), the bar says so and a notification arrives.
- A step pauses when Claude ends with an API error such as a usage limit, or when its tab is closed or restarted. **Retry** runs the step again from its start. Or help it along in its tab, after *Continue on another account* for instance: once Claude finishes an answer there, the run goes on.
- **Use last answer** finishes the current step with what Claude last said in its tab, for a step that is done without the app having noticed.
- **Stop** ends the run. Its tabs stay.

Things to know:

- A presented plan waits for approval in its tab. When the run moves on, that session is ended, so that the plan is not carried out on the planning account as well. A later step can still go on with the conversation.
- Claude Code resumes a conversation in the permission mode it was left in. A step that goes on with a plan-mode conversation stays in plan mode unless it names a mode of its own; the designer warns about that.
- A run lives as long as the app is open. After a restart its tabs come back like any others, and the run does not go on.
- One run per folder at a time; runs in different folders can go side by side.
- A step gets its prompt through a file in `~/.multi-claude/workflow-runs/`, where the outputs are kept as well, for two weeks. The first message in a step's tab names that file.
- The app notices what a step does through its hooks (see [Tabs](#tabs)). If hooks are switched off in Claude Code's settings, steps go unnoticed, and **Use last answer** is the way forward. A step cannot start at all while the app cannot open its port.
- A step that runs into a usage limit pauses; it is never moved to another account by itself.
- Workflows need the native `claude` executable; with the npm `.cmd` shim they are refused.

## Library

The book button at the foot of the sidebar opens the Library in its tab: what Claude Code reads from disk, in four sections. **Skills** and **Agents** are what it can do and whom it can hand work to, **Memory** is what it wrote down by itself, **Instructions** are your CLAUDE.md files and rules.

The sections work alike:

- Every file is edited as text and saved with **Save** or Ctrl+S. If the file changed on disk meanwhile, for example because a Claude session edited it, you choose between the version on disk and yours. Line ends and a byte order mark stay as the file had them.
- What is typed stays when another section shows or the tab is closed. A dot marks the section's tab and the sidebar button; before another file is opened and before the app quits you are asked, and the quit question names every unsaved file.
- A delete moves the file to the bin.
- Project folders come from your open tabs and recent folders. **Add project folder…** puts any other folder on the list, for all sections.

### Skills

The Skills section lists Claude Code's skills by the folder they live in and edits them:

- **Personal** skills, from each account's `skills` folder. Accounts that share the folder with the main account (the usual setup) appear as one group, named after the accounts; an account with a folder of its own gets its own group.
- **Project** skills, from `.claude/skills` in the project folders, up to the top of their repository.

Skills that come with plugins and the ones Claude Code syncs from claude.ai (`skills/synced`) are not shown and never touched.

#### Editing a skill

Choose a skill to edit its `SKILL.md`. The **Form** tab has the description, the hint for its arguments, the tools it may use, model and effort, and the instructions; the **Source** tab is the file as it is written. They are one text: a change in the form rewrites only the lines of that field, and everything else in the file (comments, fields the form does not know) stays as it was, with the file's own line ends.

- Two switches say who starts a skill: **Claude may use it by itself** and **Show it in the / menu**. The list marks skills that are "manual", "Claude only" or "off".
- Under the editor stand the things Claude Code would stumble over (a frontmatter that cannot be read, a missing description, another skill that answers to the same name) and, folded away, what claude.ai would refuse if the skill were uploaded there.
- Running sessions see a saved skill at once, though a session that has already used the skill keeps the old text.
- Files next to `SKILL.md` are listed. Text files open in their own program; anything else, scripts above all, is only shown in its folder.

#### Managing skills

- **New skill** asks for a name (lower-case letters, digits, hyphens), where it goes and a first description.
- **Rename…** renames the folder, and the name inside the file when it was the folder name. **Duplicate…** makes a copy under another name.
- **Copy / Move…** takes a skill to another folder: from the personal skills into a project or back, or to an account that has its own folder.
- **Delete** moves the folder to the bin. A skill that is a link to a folder elsewhere loses only the link.
- **Export…** writes a `.zip` with the skill as its one top folder, the form claude.ai takes; `node_modules`, `__pycache__` and the skill's own `evals` folder are left out. **Import…** reads such a file (`.zip` or `.skill`), shows what it holds, and brings it in under the name and into the folder you choose.
- **Ask Claude…** opens a session tab on the account you pick and hands it your request about the skill ("write the instructions for …", "make the description clearer"). It spends that account's messages; come back to the manager to see the result.

Things to know:

- A personal folder that several accounts share is one folder: an edit or a delete there goes for all of them.
- A personal skill wins over a project skill of the same name. The manager says so on the project skill.
- Windows may refuse to rename, move or delete a skill folder while a session works in it. A move is a copy followed by a delete, so it can end as a copy.
- The switches are written into `SKILL.md`. What Claude Code's own `/skills` menu saves in its settings can overrule them and is not shown here.
- The first `.claude/skills` folder of a project is only noticed by sessions that start afterwards, or after `/reload-skills`.
- A file whose line ends are mixed gets one kind when it is saved, and a value changed in the form may come out quoted differently than it was typed by hand.
- An imported skill can contain scripts. The app never runs them, but Claude may when it uses the skill.

### Agents

The Agents section does the same for subagents: one Markdown file each, in an account's `agents` folder (personal) or in a project's `.claude/agents`, at any depth. Claude Code knows an agent by the `name` inside its file, not by the file's name.

- The **Form** tab has the description, tools and disallowed tools, model, effort, permission mode, memory, isolation, color, the limit of turns, the skills it starts with and whether it always runs in the background; next to it stands the system prompt. The **Source** tab is the file as it is written, and both are one text, as with skills. A value the form does not offer is shown as the file has it.
- The list shows model and effort, and marks a file Claude Code does not load: one without a `name` or a `description`, or with a frontmatter that cannot be read.
- **New agent**, **Rename…** (the `name` in the file changes, and the file is renamed with it when it was named after the agent), **Duplicate…**, **Copy / Move…** between the personal agents and a project, and **Delete**.

Things to know:

- A project's agent wins over a personal one of the same name, the other way round than with skills. The manager says so on both.
- A saved agent is used the next time Claude starts it; an agent that is running keeps what it started with.
- Renaming an agent does not change the skills, workflows and prompts that call it by its old name.
- Built-in agents and those of plugins are not shown.

### Memory

Claude Code keeps an auto memory per account and project: a folder with `MEMORY.md`, the index every session reads when it starts, and one file per thing it wrote down. The Memory section lists the projects that have one, with a dot for each account that does.

- Choose a project to see its memories as the index lists them, then the files the index does not know. Any of them, the index too, can be read and edited; with several accounts a switch chooses whose memory shows.
- **New memory** makes a file with its line in `MEMORY.md`. **Delete** moves a file to the bin and takes its line out.
- The search box looks through the memories of every project and account.
- Warnings say when `MEMORY.md` is longer than the 200 lines (or 25 KB) a session reads, when it points to a file that is gone, and when a file has no line in it.
- A project is named by its folder when a conversation of it is left or the app knows the folder; otherwise by Claude Code's own name for it, the path with hyphens.

#### Sharing memory between accounts

Under the list of projects each account other than the main one has a switch: **<account> uses <main>'s memory**. Sessions the app starts for such an account (tabs, workflow steps, the external terminal) then read and write the main account's memory of the project they run in.

- Nothing is linked or moved on disk: the session is started with Claude Code's own `autoMemoryDirectory` setting. Switching it off brings the account's own memory back, for sessions that start from then on.
- Shared memory is one copy. What one account notes the others read; a delete goes for all of them.
- An account's own memories of a project are not read while it shares. **Bring into <main>'s memory…** copies them over after showing what it will do: a file the main account does not have goes as it is, one it has with another text arrives as `name.from-<account>.md`, and `MEMORY.md` gets a line for each. Nothing is removed from the account's own folder.
- The folder is worked out as Claude Code does it: the top of the git repository (for a worktree, of the repository it belongs to), with a hyphen for every character that is not a plain letter or digit. A project whose path is longer than 200 characters is not shared, and the app says so when the session starts. Should a later Claude Code name the folders differently, the app notices and stops sharing until it is updated.
- A session started outside the app, for example with `claude-<name>` in a terminal, uses the account's own memory. So does the memory a subagent keeps for itself (`agent-memory`).

### Instructions

The Instructions section edits the files you write for Claude: for an account its `CLAUDE.md` and the files under `rules`, for a project `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and the files under `.claude/rules`.

- A file that is not there yet is listed all the same and can be made with one click. **New rule** makes a rule file; a `paths` list in its frontmatter limits it to matching files, which the list shows.
- A `CLAUDE.md` over 200 lines is marked: Anthropic advises to stay under that. `@path` imports of a file are listed under its text.
- An account's `CLAUDE.md` that is copied from the main account when it is synced is not opened right away, because the next sync writes over it. You are offered the main account's file, or to edit the copy anyway.
- **What a session reads** shows, for an account and a project folder, the files Claude Code reads there in order: the account's `CLAUDE.md` and rules; then for every folder from below the top of the drive down to the project its `CLAUDE.md`, `.claude/CLAUDE.md`, rules and `CLAUDE.local.md`; imports up to four files deep; and `MEMORY.md` last. Rules with `paths` stand apart, as they are read later.

Things to know:

- Sessions read these files when they start. A session that is running keeps what it read, until `/clear` or a restart.
- The view of what a session reads is worked out from the files on disk; `/context` in a session shows what it really read. An organisation's own settings can leave files out.
- The walk through the folders includes your home folder. A session of another account that runs below it therefore also reads the main account's `~/.claude/CLAUDE.md`, as a file of that folder.

## Tray icon

The app keeps an icon in the system tray (the menu bar on macOS). Closing the window only hides it: the sessions keep running, and notifications still arrive. Click the icon to get the window back. Its right-click menu shows each account's usage and has **New session…** and **Quit**; quitting asks first while sessions are running.

- Settings → **Window → Keep running in the tray when the window is closed** switches this off. Closing the window then quits the app.
- **Start with Windows** ("Start with the system" on macOS; installed app only) starts the app in the tray when you log in. On Windows it writes the value `Multi Claude` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which the uninstaller removes; on macOS it is a launch agent. The tabs of the last run are reopened when the window is first shown, not at login.
- `"Multi Claude.exe" --quit` ends a running instance from a script, without asking.
- Running an installer by hand while the app sits in the tray ends it the hard way; the tabs come back from what was last saved. **Install and restart** (see [Updates](#updates)) quits the app properly first.

## Usage indicator

Each card shows the account's 5-hour session limit ("5h"), its weekly limit ("Week") and, where one exists, a model-specific weekly limit (for example "Fable"). The numbers come from two places:

- **Claude Code's `/usage` command:** the app runs `claude -p "/usage"` for every account: at startup, every 10 minutes, when the window comes to the front, when a session ends, and when you click the refresh icon on a card. It makes no model call, and it keeps accounts without an open session up to date as well.
- **Status line:** while sessions are running, the 5-hour and weekly percentages update live. Your own status line keeps working as before.

A plan without a 5-hour limit (Ultra) reports weekly numbers only. Its card shows just the weekly rows, the **+** shows its weekly limit, and when the app suggests an account to continue on, it counts as having all the room of a 5-hour limit.

**Limit alerts.** When an account's 5-hour limit, weekly limit or a model's weekly limit passes 80% and again at 95%, the app says so: as a desktop notification, or as a line inside the window when the window is in front. It also tells you when a limit it warned about has reset. Numbers that are already high when the app first sees them (at startup, or usage built up on another machine) are not announced. Settings → **Accounts** switches the alerts off.

Hover a row to see when it resets and how old the data is; the account menu shows how long ago the data was last updated. The app does not read OAuth tokens; Claude Code itself fetches the usage.

## Explorer context menu (Windows)

Settings → **Explorer context menu → Add** puts "Open in Multi Claude" into the menu you get by right-clicking a folder or a folder's background. If the app is running, the folder opens in a new tab; otherwise the app starts. With more than one logged-in account, the app asks which one to use. If the entry does not show up, restart File Explorer.

- **Pinning a folder to an account:** tick "Always use this account for this folder" when the app asks, or use **Always open this folder with …** in a tab's right-click menu. From then on the folder opens with that account without asking, and the folder list of the **+** shows the account next to it (choosing an account there by hand still wins). The same menu entry removes the pin.
- **Windows 11 menu:** the entry shows in the new menu for the installed app when Developer Mode is on. This registers the unsigned sparse package from the app's `explorer` folder (`src-tauri/resources/explorer` in the repository); `mc_explorer_command_x64.dll` provides the command.
- **Classic menu:** without Developer Mode, on Windows 10, when Windows is set to the classic menu, or when running with `npm run tauri dev`, the entry is written to the registry and shows under "Show more options". A development run writes its own entry, "Open in Multi Claude (Dev)", and leaves the installed app's alone.
- Uninstalling the app removes the entry. After installing an update, the entry comes back the first time the app starts.

The DLL is written in Rust (`crates/mc-explorer-command`) and kept built in the repository, so ordinary builds do not make it. `npm run build:shell` rebuilds it and checks that it is an x64 DLL with the two COM exports and only Windows' own DLLs as imports; it needs Visual Studio Build Tools for `dumpbin`. When the DLL or `AppxManifest.xml` changes, bump `Version` in the manifest.

## Shortcuts

| Key | Action |
| --- | --- |
| Ctrl+Shift+T | New session (pick account and folder; same as the + in the tab bar) |
| Ctrl+Shift+W | Close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab, through all areas |
| Ctrl+1 … Ctrl+8 | Go to that tab of the area that has the keyboard |
| Ctrl+9 | Go to its last tab |
| Ctrl+Shift+D | Give the active tab an area of its own on the right |
| F6 / Shift+F6 | Move the keyboard to the next / previous area |
| Ctrl+Shift+H | Find a past conversation |
| Ctrl+Shift+F | Find in the terminal |
| Ctrl+Shift+Z | Put the active tab to sleep, or wake it |
| Ctrl+= / Ctrl+- / Ctrl+0 | Larger / smaller / default terminal font; Ctrl+wheel does the same |
| Ctrl+Shift+B | Collapse or expand the sidebar |
| Ctrl+S | In the Library: save the file in the editor |
| Esc | Close the Workflows or Library tab while it has the keyboard |
| Shift+Enter | New line without sending |
| Ctrl+V, right-click | Paste |
| Ctrl+C (with a selection), right-click | Copy |

Every other key goes to Claude Code. Ctrl+- is the one key the app takes that a terminal could pass on, as Windows Terminal does too; Claude Code's undo remains on Ctrl+Shift+- (Ctrl+_).

## Files

| Location | Contents |
| --- | --- |
| `~/.multi-claude/config.json` | Profiles (with whether an account shares the main account's memory), settings, recent folders, folders pinned to an account, project folders added to the Library |
| `~/.multi-claude/profiles/` | Claude Code folders of the accounts other than the main one |
| `~/.multi-claude/bin/claude-<name>.cmd` | Launchers that start an account from any terminal (without `.cmd` on macOS) |
| `~/.multi-claude/tabs.json` | The open tabs and their arrangement, for the next start; a tab that was asleep carries `"asleep": true` and comes back asleep |
| `~/.multi-claude/workflows.json` | The designed workflows |
| `~/.multi-claude/workflow-runs/` | Prompts and outputs of the steps of recent workflow runs |
| `~/.multi-claude/skill-requests/` | What Claude was asked about a skill, for two weeks |
| `~/.multi-claude/limit-events.json` | The last twenty failed turns and ended limit waits, as Claude Code reported them: the error's name and the beginning of its text |
| `~/.multi-claude/run/` | Temporary files of running sessions |

The tokens of token accounts are not in these files but in the credential store. Updates keep no files of their own.

## Updates

The installed app updates itself from the GitHub Releases of this repository. It reads `latest.json` of the newest published release 10 seconds after it starts and then once a day. Settings → **Window → Check for updates automatically** turns that off, and **Check now** looks right away. A development run (`npm run tauri dev`) and an isolated instance (`MULTI_CLAUDE_HOME`) never check by themselves, so that they cannot install over the real app.

When a newer version is found, a bar above the tabs offers **What's new**, **Install and restart** and **Later**; after **Later**, the `↑` next to the version at the foot of the sidebar brings the bar back. Installing downloads the update, checks its signature, ends the app the way quitting does (the tabs are saved, the sessions end), installs it and starts the new version, which reopens the tabs. On Windows the NSIS installer runs in passive mode, for the current user, without an administrator prompt; on macOS the new app bundle takes the place of the old one.

### One-time setup

1. **Repository.** Releases come from the public repository `OWNER/multi-claude-rust`. Put the real owner in place of `OWNER` in `src-tauri/tauri.conf.json` → `plugins.updater.endpoints`; the app takes it from nowhere else. The repository must be public, since the app downloads without credentials.
2. **Signing key.** Generate the key pair once, and choose a password when asked:

   ```sh
   npx tauri signer generate -w ~/.tauri/multi-claude.key
   ```

   This writes the private key `~/.tauri/multi-claude.key` and the public key `~/.tauri/multi-claude.key.pub`. Keep the private key and its password safe and out of the repository. Without them no later update can be signed, and installed apps then have to be updated by hand with a new installer.
3. **Public key.** Paste the content of `multi-claude.key.pub` (one line) into `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, in place of `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY`, and commit it. Apps built before this step cannot check for any update.
4. **Secrets.** On GitHub, under the repository's Settings → Secrets and variables → Actions, add the repository secrets `TAURI_SIGNING_PRIVATE_KEY` (the whole content of `multi-claude.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the password from step 2). GitHub provides `GITHUB_TOKEN`. The release workflow asks for `contents: write`; if the release cannot be created because Settings → Actions → General → Workflow permissions are read-only, allow read and write there.

### Making a release

1. Raise the version in `src-tauri/tauri.conf.json` (the app reports this one), and to keep them in step, in `package.json` and the workspace `Cargo.toml`.
2. Add a section `## [x.y.z] - YYYY-MM-DD` to `CHANGELOG.md`. Its text becomes the release text and what the app shows under **What's new** before installing.
3. Commit, then tag and push the tag. The tag must be `v` plus the version of `tauri.conf.json`; the workflow stops otherwise.

   ```sh
   git tag v0.6.1
   git push origin v0.6.1
   ```

4. `.github/workflows/release.yml` builds Windows x64, macOS Apple silicon and macOS Intel one after the other, and puts the installers, their `.sig` files and `latest.json` into a **draft** release.
5. Check that the draft's `latest.json` lists `windows-x86_64`, `darwin-aarch64` and `darwin-x86_64`, then publish it. Installed apps see the update from that moment; a draft or a prerelease is never offered.

`.github/workflows/ci.yml` runs the type check, the window's tests, a build of the window and the tests of the Rust workspace on every push and pull request (Windows only).

### Building locally

`npm run tauri build` and `npm run dist` sign the updater files. They fail without the private key, and also while `plugins.updater.pubkey` still holds the placeholder ("failed to decode pubkey"). Set the key for the build:

```sh
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/multi-claude.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="…" npm run dist
```

For a build without updater files, which cannot be published as an update:

```sh
npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

## Project layout

| Path | Contents |
| --- | --- |
| `src/` | The window: React components, stores, the xterm.js terminals, and `api.ts`, the bridge to the backend |
| `src/shared/` | Logic and types the window's code shares; `ipc.ts` lists the channels to the backend |
| `test/` | The window's tests |
| `crates/mc-core/` | Pure Rust logic (types, validation, algorithms) with the test suites ported from the Electron version; no window, terminal, process or network |
| `src-tauri/` | The app: sessions, hook server, usage, the Library's files, tray, updater, Explorer menu. Commands in `src-tauri/src/commands/`, uninstaller hooks in `src-tauri/nsis/hooks.nsh` |
| `crates/mc-explorer-command/` | The Explorer context menu DLL, built by `scripts/build-explorer-dll.ps1` |
| `.github/workflows/` | `ci.yml` (tests on every push and pull request), `release.yml` (the installers of a version tag) |

Each Tauri command is the IPC channel the window calls, in snake_case: `profiles:create` is `profiles_create`, `app:getState` is `app_get_state`. `src/api.ts` makes that call and lists the names of each channel's arguments, which the Rust commands use as well. Events keep the channel names (`pty:data`).

## Notes

- Every session starts in an environment stripped of account variables such as `CLAUDE_CODE_OAUTH_TOKEN` and `ANTHROPIC_API_KEY`; the account is selected with `CLAUDE_CONFIG_DIR` only.
- The Electron version and this one do not know of each other's single-instance lock. Run one of them at a time, since both work on the same files.
- The macOS package has to be built on a Mac; the release workflow builds it on GitHub's macOS runners. It is signed ad hoc and not notarized, so macOS refuses to open a downloaded copy until it is allowed under System Settings → Privacy & Security.
- The Rust version has only been run on Windows so far. Its macOS code paths (the menu bar icon, the Dock icon bringing the window back, the Keychain, the start with the system, the update that replaces the app bundle) are written, but have not been tried on a Mac.
