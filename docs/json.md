# chatkeep JSON output

`--json` makes chatkeep print exactly one JSON document on stdout, for apps and scripts that run
the `chatkeep` binary as a subprocess. This file is the contract: field names are snake_case and
stable, and every command an app may call is described below with its exact `data` shape.

```bash
chatkeep ls --json
chatkeep --json mv -n ~/old/app ~/new/app
```

`--json` is a global flag: it may stand anywhere before a `--`. Without it, every command prints
text exactly as before.

## What `--json` changes

- **One document on stdout.** The document is one line of JSON followed by a newline. Nothing
  else is written to stdout: no tables, spinners, progress bars, colors, banners, update notices,
  or hint lines. stderr can still carry a warning line (for example when the index cannot be
  opened); never parse stderr.
- **It never asks.** A run with `--json` is never interactive, also in a terminal. A command that
  would ask for a missing argument, or show a picker, fails with `missing_argument` instead.
- **Writes need `-n` or `-y`.** A command that changes data and takes `-y` fails with
  `needs_confirmation` unless it gets `-n` (show the plan, change nothing) or `-y` (run it). See
  [Writes](#writes).
- **History.** Every write that a run without `--json` logs in chatkeep's command log is logged
  the same way: the [writes](#writes) that take `-y`, `claude accounts name`,
  `claude sync auto on|off`, and `cursor cache scan` (it rewrites the index), each only when it is
  no dry run. The queue commands are not logged, as without `--json`. Read commands (`ls`, `ls ID`,
  `stats`, `cursor cache stats`, `cursor cache scan -n`, and the `claude` read commands) are not
  logged, unlike a text run, so an app that polls does not fill the log.

## Envelope

### Success

```json
{
  "ok": true,
  "command": "ls",
  "dry_run": false,
  "results": [
    {"tool": "claude", "data": {}},
    {"tool": "cursor", "data": {}}
  ]
}
```

| Field     | Type    | Meaning                                                                   |
| --------- | ------- | ------------------------------------------------------------------------- |
| `ok`      | bool    | Always `true`.                                                            |
| `command` | string  | The command's words, see [Command names](#command-names).                 |
| `dry_run` | bool    | `true` when `-n` was given.                                               |
| `results` | array   | One entry per tool the command ran for, in the order they ran.           |

Each result is `{"tool": "cursor" | "claude" | null, "data": {...}}`.

**The `tool` rule:** `tool` names the tool whose data the result is about. Commands that run per
tool (`ls`, `stats`, `mv`, `cp`, `rm`, `split`, `combine`, `export`, `import`) give one result
per tool they ran for; with both tools that is Claude Code first, then Cursor. Every
`chatkeep cursor ...` command gives one result with `"cursor"`, every `chatkeep claude ...`
command one with `"claude"`. Commands about chatkeep's own state (`history`, every `queue`
command, `--version`) give one result with `null`.

### Error

```json
{
  "ok": false,
  "command": "mv",
  "error": {
    "kind": "tool_running",
    "message": "Cursor is running: default (Cursor, pid 4242).",
    "hint": "Queue it with: chatkeep queue add -- mv -y /Users/me/old/app /Users/me/new/app",
    "queue_args": ["mv", "-y", "/Users/me/old/app", "/Users/me/new/app"]
  },
  "results": []
}
```

| Field              | Type             | Meaning                                                                                  |
| ------------------ | ---------------- | ---------------------------------------------------------------------------------------- |
| `ok`               | bool             | Always `false`.                                                                          |
| `command`          | string or null   | The command's words; `null` when the arguments could not be parsed.                     |
| `error.kind`       | string           | One of the [error kinds](#error-kinds).                                                  |
| `error.message`    | string           | What went wrong, in one line, for people.                                                |
| `error.hint`       | string or null   | What to do about it, for people.                                                         |
| `error.queue_args` | array or null    | Only with `tool_running`: the arguments `queue add` would store, see below.              |
| `results`          | array            | The parts that finished before the error, in the result shape above. Usually empty.     |

`results` is not empty in two cases: a command that ran for both tools and failed in its Cursor
part after its Claude Code part was done (the Claude Code result is there), and `queue execute`,
whose result describes every entry also when the run stopped early.

### Exit codes

| Code | When                                                              |
| ---- | ----------------------------------------------------------------- |
| `0`  | `ok` is `true`.                                                   |
| `1`  | Any error except the two below.                                   |
| `2`  | `usage`: the arguments could not be parsed.                       |
| `75` | `tool_running`: Cursor or Claude Code still uses the data.        |

### Error kinds

| Kind                  | Meaning                                                                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `usage`               | clap could not parse the arguments: an unknown command or flag, a bad value. `command` is `null`. Also for a `queue add` whose stored command does not parse. |
| `missing_argument`    | The command needs something a terminal would have asked for: an argument, a pick from a list, or `--profile` when several Cursor installations hold the project. The message names it. |
| `needs_confirmation`  | A write command got neither `-n` nor `-y`.                                                                                                                    |
| `tool_running`        | Cursor is open, or a Claude Code session or the Claude desktop app uses the chats. Nothing was changed by the part that hit it.                             |
| `unsupported_in_json` | The command has no JSON output: `update`, `uninstall`, `github`, `help` (and `-h`), `claude sync watch`, `__refresh-index`.                                  |
| `failed`              | Anything else: no project matches, an archive is broken, a refused combination of flags. Read `message`.                                                     |

**Queueing a blocked command.** With `tool_running`, `queue_args` is non-null when the command
can be queued: run `chatkeep --json queue add -- <queue_args...>` and run
`chatkeep --json queue execute -y` once the tool is closed. `queue_args` is the command as it was
given (without `--json`). When its Claude Code part already ran, `--tool cursor` is added so the
queue runs only Cursor's part: it goes before the first `--` (or replaces the value of a `--tool`
already given), so positionals after `--` stay positionals. For example
`mv -y -- /x/a /x/b` gives `["mv", "-y", "--tool", "cursor", "--", "/x/a", "/x/b"]`. A
`queue execute` that stops after an entry's Claude Code part stores the entry's command the
same way. `queue_args` is `null` when `queue execute` itself was blocked: the entries are
already queued.

## Command names

`command` is the command's words with aliases resolved: `ls`, `mv`, `cp`, `split`, `combine`,
`rm`, `export`, `import`, `history`, `stats`, `queue add`, `queue list`, `queue rm`,
`queue clear`, `queue retry`, `queue execute`, `cursor save`, `cursor rx`, `cursor cache clear`,
`cursor cache scan`, `cursor cache stats`, `claude accounts ls`, `claude accounts cp`,
`claude accounts name`, `claude accounts doctor`, `claude accounts prune`, `claude cache clear`,
`claude cache stats`, `claude sync` (running a sync), `claude sync profiles`, `claude sync set`,
`claude sync add`, `claude sync rm`, `claude sync log`, `claude sync auto on`,
`claude sync auto off`, `claude sync auto status`, and `version`.

## Values

| Kind of value | Format                                                                                                                         |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Time          | ISO 8601 in UTC, whole seconds: `"2026-10-27T23:21:38Z"`. `null` when unknown. Swift: `JSONDecoder.DateDecodingStrategy.iso8601`. |
| Path          | Absolute path string as the file system spells it.                                                                             |
| Size          | Integer bytes; the field name ends in `_bytes` (or is `bytes`).                                                                |
| Duration      | Integer milliseconds; the field name ends in `_ms`.                                                                            |
| Tokens, counts| Integers.                                                                                                                      |
| `name`        | A display label, as the text tables show it: a project path with the home folder as `~`. Use `path` for the real path.         |

A field documented as "or null" is always present, with `null` when it has no value. Fields are
never left out, except that rows of one tool have a few fields the other tool's rows do not
(listed per shape). New fields may be added; existing ones keep their name and type.

## Reads

### `ls`

`chatkeep ls --json [--tool TOOL] [--profile NAME] [--unsaved] [--fresh]`

One result per tool. `data`:

| Field              | Type            | Meaning                                                                                                 |
| ------------------ | --------------- | ------------------------------------------------------------------------------------------------------- |
| `index_as_of`      | time or null    | Cursor: when the index last read the rows, when they came from the index. Always `null` for Claude Code. |
| `index_refreshing` | bool            | Cursor: a background refresh of the index runs now. Always `false` for Claude Code.                     |
| `projects`         | array of [Project](#project) | Missing destinations first, then the largest. Sort by `last_active` in the app for a "recent first" view. |

### `ls ID`

`chatkeep ls --json ID` (ID: a folder path, a Cursor workspace hash, or a Claude Code project folder name)

One result per tool that has the project. `data`:

| Field              | Type    | Meaning                                                                  |
| ------------------ | ------- | ------------------------------------------------------------------------ |
| `index_as_of`      | time or null | As for `ls`.                                                        |
| `index_refreshing` | bool    | As for `ls`.                                                             |
| `details`          | array   | One entry per Cursor installation that has the project (Claude Code: one). |

Each entry of `details`:

| Field         | Type                   | Meaning                                                  |
| ------------- | ---------------------- | -------------------------------------------------------- |
| `project`     | [Project](#project)    | The project row.                                         |
| `last_active` | time or null           | The same as `project.last_active`.                       |
| `chats`       | array of [Chat](#chat) | Every chat, newest first.                                |

### Project

| Field                 | Type           | Cursor                                                                        | Claude Code                                       |
| --------------------- | -------------- | ----------------------------------------------------------------------------- | ------------------------------------------------- |
| `tool`                | string         | `"cursor"`                                                                    | `"claude"`                                        |
| `id`                  | string         | Workspace hash (32 hex characters), or the id of an unsaved or empty window   | Folder name in `~/.claude/projects`               |
| `kind`                | string         | `folder`, `code-workspace`, `unsaved`, `empty-window`, or `remote`            | `project`                                         |
| `name`                | string         | Display label                                                                 | Display label                                     |
| `path`                | string or null | The folder or `.code-workspace` file                                          | The folder, when chatkeep could tell              |
| `uri`                 | string or null | The URI Cursor stores                                                         | `null`                                            |
| `storage_dir`         | string         | `workspaceStorage/<id>` folder                                                | The project folder in `~/.claude/projects`        |
| `install`             | string or null | Cursor installation name, e.g. `default`                                      | `null`                                            |
| `profile`             | string or null | VS Code profile, `default` for the default one                                | `null`                                            |
| `profile_label`       | string or null | `install`, or `install/profile` for another VS Code profile                    | `null`                                            |
| `chats`               | int            | Top-level chats                                                               | Sessions                                          |
| `subagents`           | int            | Subagent chats                                                                | Subagent transcripts                              |
| `archived`            | int or null    | Archived chats (top-level and subagent)                                       | `null`                                            |
| `size_bytes`          | int            | Size of `storage_dir`                                                         | Size of `storage_dir`                             |
| `last_active`         | time or null   | Newest `updated_at` of its chats, subagent chats included (`created_at` for a chat with no `updated_at`); `null` without chats | Newest change of its session transcripts; `null` without sessions |
| `destination`         | string         | `present`, `missing`, or `none` (a kind with no folder of its own)            | `present`, `missing`, or `none` (no known path)   |
| `destination_missing` | bool           | `destination == "missing"`                                                    | `destination == "missing"`                        |
| `memory_files`        | int            | (not present)                                                                 | Files in the project's `memory/` folder           |

### Chat

| Field         | Type            | Cursor                                                 | Claude Code                                                          |
| ------------- | --------------- | ------------------------------------------------------ | -------------------------------------------------------------------- |
| `id`          | string          | Chat (composer) id                                     | Session id                                                           |
| `title`       | string or null  | Chat title                                             | Custom title                                                         |
| `created_at`  | time or null    | When the chat was created                              | `null` (not recorded)                                                |
| `updated_at`  | time or null    | Last update of the chat                                | Last change of the transcript file                                   |
| `archived`    | bool or null    | Archived in Cursor                                     | `null`                                                               |
| `subagent`    | bool            | A chat an agent started                                | `false`                                                              |
| `model`       | string or null  | Model of the chat, when Cursor recorded it             | The model that gave most answers                                     |
| `tokens`      | object or null  | `{"input", "output", "context"}`, `null` when Cursor has no data for the chat | `{"input", "output", "cache_write", "cache_read"}` |
| `cost_cents`  | int or null     | Usage cost Cursor recorded                             | (not present)                                                        |
| `requests`    | int or null     | Requests Cursor recorded                               | (not present)                                                        |
| `subagents`   | int             | (not present)                                          | Subagent transcripts of the session                                  |
| `size_bytes`  | int             | (not present)                                          | Transcript plus its session folder                                   |
| `cwd`         | string or null  | (not present)                                          | The folder the session started in                                    |
| `transcript`  | string          | (not present)                                          | The `.jsonl` transcript                                              |

Cursor `tokens.input` and `tokens.output` only count tokens stored inline in the chat; newer
chats store them per message, so they are often `0`. `tokens.context` is the context window in
use at the last turn.

Example (`chatkeep ls --json ~/projects/resolved/api`, one chat kept per tool):

```json
{
  "ok": true,
  "command": "ls",
  "dry_run": false,
  "results": [
    {
      "tool": "claude",
      "data": {
        "index_as_of": null,
        "index_refreshing": false,
        "details": [
          {
            "project": {
              "tool": "claude",
              "id": "-Users-me-projects-resolved-api",
              "kind": "project",
              "name": "~/projects/resolved/api",
              "path": "/Users/me/projects/resolved/api",
              "uri": null,
              "storage_dir": "/Users/me/.claude/projects/-Users-me-projects-resolved-api",
              "install": null,
              "profile": null,
              "profile_label": null,
              "chats": 43,
              "subagents": 200,
              "archived": null,
              "size_bytes": 439708356,
              "last_active": "2026-10-27T23:22:02Z",
              "destination": "present",
              "destination_missing": false,
              "memory_files": 17
            },
            "last_active": "2026-10-27T23:22:02Z",
            "chats": [
              {
                "id": "88effc4d-8752-4607-8eb9-5d4bf880a8d3",
                "title": "Fix login redirect",
                "created_at": null,
                "updated_at": "2026-10-27T23:22:02Z",
                "archived": null,
                "subagent": false,
                "model": "claude-opus-5-5",
                "tokens": {"input": 304, "output": 173503, "cache_write": 269684, "cache_read": 28345607},
                "subagents": 8,
                "size_bytes": 10830749,
                "cwd": "/Users/me/projects/resolved/api",
                "transcript": "/Users/me/.claude/projects/-Users-me-projects-resolved-api/88effc4d-8752-4607-8eb9-5d4bf880a8d3.jsonl"
              }
            ]
          }
        ]
      }
    },
    {
      "tool": "cursor",
      "data": {
        "index_as_of": "2026-10-27T23:21:38Z",
        "index_refreshing": false,
        "details": [
          {
            "project": {
              "tool": "cursor",
              "id": "0fe570bb46aa058cdd9bdfcf83698bfb",
              "kind": "folder",
              "name": "~/projects/resolved/api",
              "path": "/Users/me/projects/resolved/api",
              "uri": "file:///Users/me/projects/resolved/api",
              "storage_dir": "/Users/me/Library/Application Support/Cursor/User/workspaceStorage/0fe570bb46aa058cdd9bdfcf83698bfb",
              "install": "default",
              "profile": "default",
              "profile_label": "default",
              "chats": 1,
              "subagents": 0,
              "archived": 0,
              "size_bytes": 89225785,
              "last_active": "2026-09-24T13:33:37Z",
              "destination": "present",
              "destination_missing": false
            },
            "last_active": "2026-09-24T13:33:37Z",
            "chats": [
              {
                "id": "75b81147-47da-4df0-b342-bdd8bfc2e5cd",
                "title": "Add the export button",
                "created_at": "2026-09-24T13:33:34Z",
                "updated_at": "2026-09-24T13:33:37Z",
                "archived": false,
                "subagent": false,
                "model": "grok-4.7",
                "tokens": {"input": 0, "output": 0, "context": 156861},
                "cost_cents": 0,
                "requests": 0
              }
            ]
          }
        ]
      }
    }
  ]
}
```

`chatkeep ls --json` without an ID has the same `index_as_of` and `index_refreshing`, and
`projects` holds the bare [Project](#project) rows, `last_active` included. For Cursor, rows
read from the index carry the times the index holds as of `index_as_of`; the background refresh
(or `--fresh`) brings them up to date.

### `stats`

`chatkeep stats --json [--tool TOOL] [--profile NAME] [--fresh]`

One result per tool.

**Cursor `data`:**

| Field              | Type          | Meaning                                                                                       |
| ------------------ | ------------- | --------------------------------------------------------------------------------------------- |
| `index_as_of`      | time or null  | As for `ls`.                                                                                  |
| `index_refreshing` | bool          | As for `ls`.                                                                                  |
| `installations`    | int           | Cursor installations counted.                                                                 |
| `user_profiles`    | int           | Extra VS Code profiles in them.                                                               |
| `profiles`         | array         | `{"name", "root", "workspaces", "chats", "subagents", "global_db_bytes", "user_profile"}` per installation, then per VS Code profile. `root` is the installation's user data folder; `global_db_bytes` is `null` for a VS Code profile (it shares the installation's database). |
| `workspaces`       | int           | Workspaces.                                                                                   |
| `by_kind`          | array         | `{"kind", "workspaces"}`, most first.                                                         |
| `chats`            | int           | Top-level chats.                                                                              |
| `subagents`        | int           | Subagent chats.                                                                               |
| `archived`         | int           | Archived chats and subagent chats.                                                            |
| `usage`            | object        | `conversations`, `cost_cents`, `requests`, `cost_chats`, `context_tokens`, `context_chats`, `input_tokens`, `output_tokens`, `token_chats`: totals, and how many conversations carried each. |
| `models`           | array         | `{"model", "chats"}`, most first.                                                             |
| `per_month`        | array         | `{"month": "YYYY-MM", "chats"}` by creation month, oldest first.                              |
| `busiest`          | array         | `{"name", "kind", "chats"}` per workspace, most chats first. `kind` may be `null`.           |
| `global_db_bytes`  | int or null   | Size of every `globalStorage/state.vscdb` together.                                           |
| `workspace_dbs`    | array         | `{"name", "kind", "bytes"}` per workspace database, largest first.                            |

`name` in `busiest` and `workspace_dbs` is a display label; with several installations it ends in
` (installation)`.

**Claude Code `data`:**

| Field             | Type   | Meaning                                                                          |
| ----------------- | ------ | -------------------------------------------------------------------------------- |
| `projects`        | int    | Projects.                                                                        |
| `missing`         | int    | Projects whose folder is gone.                                                   |
| `sessions`        | int    | Sessions.                                                                        |
| `subagents`       | int    | Subagent transcripts.                                                            |
| `memory_files`    | int    | Memory files.                                                                    |
| `size_bytes`      | int    | Size of every project folder.                                                    |
| `tokens`          | object | `{"input", "output", "cache_write", "cache_read"}` of the sessions.              |
| `subagent_tokens` | object | The same for the subagent transcripts.                                           |
| `models`          | array  | `{"model", "sessions"}`: sessions by the model that gave most answers, most first. |
| `per_month`       | array  | `{"month": "YYYY-MM", "sessions"}`, oldest first.                                |
| `busiest`         | array  | `{"name", "sessions"}` per project, most first.                                  |
| `largest`         | array  | `{"name", "size_bytes"}` per project, largest first.                             |

Example (arrays cut to one entry):

```json
{
  "ok": true,
  "command": "stats",
  "dry_run": false,
  "results": [
    {
      "tool": "claude",
      "data": {
        "projects": 40, "missing": 18, "sessions": 82, "subagents": 769, "memory_files": 32,
        "size_bytes": 1847276300,
        "tokens": {"input": 277412, "output": 21661122, "cache_write": 114940596, "cache_read": 10853438900},
        "subagent_tokens": {"input": 224672, "output": 9228788, "cache_write": 776797006, "cache_read": 18135669495},
        "models": [{"model": "claude-opus-5-5", "sessions": 71}],
        "per_month": [{"month": "2026-09", "sessions": 5}],
        "busiest": [{"name": "~/projects/resolved/api", "sessions": 43}],
        "largest": [{"name": "~/Library/Application Support/Claude/scratch-workspaces/46b3b10a/scratch-2026-10-07-8b8195", "size_bytes": 659715219}]
      }
    },
    {
      "tool": "cursor",
      "data": {
        "index_as_of": "2026-10-27T23:21:38Z",
        "index_refreshing": false,
        "installations": 8,
        "user_profiles": 0,
        "profiles": [
          {"name": "default", "root": "/Users/me/Library/Application Support/Cursor", "workspaces": 73, "chats": 1144, "subagents": 1049, "global_db_bytes": 34357874688, "user_profile": false}
        ],
        "workspaces": 184,
        "by_kind": [{"kind": "folder", "workspaces": 111}],
        "chats": 3620,
        "subagents": 12363,
        "archived": 2233,
        "usage": {
          "conversations": 15983, "cost_cents": 61466, "requests": 9476, "cost_chats": 213,
          "context_tokens": 1620491356, "context_chats": 13623,
          "input_tokens": 21433261, "output_tokens": 1197591, "token_chats": 60
        },
        "models": [{"model": "grok-4.6", "chats": 781}],
        "per_month": [{"month": "2025-02", "chats": 25}],
        "busiest": [{"name": "~/projects/verbleif/verbleif.code-workspace (default)", "kind": "code-workspace", "chats": 1092}],
        "global_db_bytes": 158142390272,
        "workspace_dbs": [{"name": "~/projects/verbleif/frontend (default)", "kind": "folder", "bytes": 19136512}]
      }
    }
  ]
}
```

### `history`

`chatkeep history --json`

One result, `tool: null`. `data`:

| Field        | Type   | Meaning                                                                 |
| ------------ | ------ | ----------------------------------------------------------------------- |
| `path`       | string | The log file.                                                           |
| `entries`    | array  | Oldest first. `{"at", "command", "args", "outcome", "duration_ms"}`.   |
| `unreadable` | int    | Lines that are not JSON; they are left out.                             |

Per entry: `at` time or null; `command` string or null (`mv`, `claude accounts`, ...); `args`
array of strings with empty ones left out; `outcome` `"ok"` or `"error"` (or null);
`duration_ms` int or null.

```json
{"ok":true,"command":"history","dry_run":false,"results":[{"tool":null,"data":{"path":"/Users/me/.chatkeep/history.jsonl","entries":[{"at":"2026-09-24T10:04:28Z","command":"mv","args":["/Users/me/old/app","/Users/me/new/app"],"outcome":"ok","duration_ms":162}],"unreadable":0}}]}
```

### `queue list`

`chatkeep queue list --json`

One result, `tool: null`. `data` is `{"entries": [Entry, ...]}` in queue order.

**Entry:**

| Field          | Type            | Meaning                                                                                       |
| -------------- | --------------- | --------------------------------------------------------------------------------------------- |
| `id`           | string          | Entry id, 8 hex characters.                                                                    |
| `position`     | int or null     | Run order among the pending entries, from 1; `null` when not pending.                         |
| `status`       | string          | `pending`, `running`, `failed`, or `skipped`.                                                 |
| `interrupted`  | bool            | `failed` because a `queue execute` stopped while it ran.                                      |
| `argv`         | array of string | The stored command, without `chatkeep`.                                                       |
| `command`      | string          | `argv` quoted for a shell.                                                                    |
| `tool`         | string or null  | `cursor` or `claude` when the command names its tool (its namespace, `--tool`, `--profile`, `--unsaved`); `null` when chatkeep decides when it runs. |
| `cwd`          | string          | The folder it was queued in; relative paths in `argv` resolve against it.                     |
| `profile`      | string or null  | `--profile` of the command.                                                                   |
| `created_at`   | time or null    | When it was queued.                                                                           |
| `tool_version` | string          | chatkeep version that queued it.                                                              |
| `warnings`     | array of string | Warnings of its plan when it was queued.                                                      |
| `last_error`   | string or null  | Why it failed or was skipped.                                                                 |

```json
{"ok":true,"command":"queue list","dry_run":false,"results":[{"tool":null,"data":{"entries":[{"id":"de1cd8d8","position":null,"status":"failed","interrupted":false,"argv":["combine","--move","--profile","resolved","b42728393cad7ca6161e8ee23cf1cc08","7b86a000ce7c6377c39429a3bd7e2080","-y"],"command":"combine --move --profile resolved b42728393cad7ca6161e8ee23cf1cc08 7b86a000ce7c6377c39429a3bd7e2080 -y","tool":"cursor","cwd":"/Users/me/projects/chatkeep","profile":"resolved","created_at":"2026-10-01T10:27:20Z","tool_version":"1.1.0","warnings":[],"last_error":"collision: transcript already exists: /Users/me/.cursor/projects/x/agent-transcripts/24f3dae7\nAll changes were rolled back."}]}}]}
```

### `queue add`

`chatkeep --json queue add -- COMMAND [ARGS...]`

Validates the command with a dry run and stores it. It does not take `-y`. One result,
`tool: null`; `data` is `{"entry": Entry}` with `position` set. It fails with `failed` when the
command would not wait for a tool ("can run directly") or its dry run fails, and with `usage`
when the stored command does not parse.

```json
{"ok":true,"command":"queue add","dry_run":false,"results":[{"tool":null,"data":{"entry":{"id":"3f9c2a1b","position":1,"status":"pending","interrupted":false,"argv":["mv","-y","/Users/me/old/app","/Users/me/new/app"],"command":"mv -y /Users/me/old/app /Users/me/new/app","tool":null,"cwd":"/Users/me","profile":null,"created_at":"2026-10-28T09:30:00Z","tool_version":"1.2.0","warnings":[],"last_error":null}}}]}
```

### `cursor cache stats`

`chatkeep cursor cache stats --json [--profile NAME]`

One result, `tool: "cursor"`. `data`:

| Field          | Type            | Meaning                                                                       |
| -------------- | --------------- | ----------------------------------------------------------------------------- |
| `path`         | string          | `index.db`.                                                                   |
| `exists`       | bool            | Whether it exists.                                                            |
| `size_bytes`   | int             | Its size with `-wal` and `-shm`.                                              |
| `schema`       | int or null     | Its schema version.                                                           |
| `refreshed_at` | time or null    | Last refresh.                                                                 |
| `spawned_at`   | time or null    | Last background refresh start.                                                |
| `refresh`      | object or null  | The lock holder: `{"pid", "started_at", "alive"}`; `null` when unlocked.      |
| `last_log`     | string or null  | Last line of the background refresh log.                                      |
| `installs`     | array           | Per installation, see below.                                                  |

Per install: `name`; `root` (user data folder); `found` (false for rows of an installation that
is no longer on this machine); `scanned_at`, `dirty_at` (time or null); `scan_duration_ms` (int
or null); `workspaces`, `chats`, `subagents` (ints); `stale` (`{"sources", "reasons"}` or null
when unknown).

```json
{"ok":true,"command":"cursor cache stats","dry_run":false,"results":[{"tool":"cursor","data":{"path":"/Users/me/.chatkeep/index.db","exists":true,"size_bytes":7409664,"schema":1,"refreshed_at":"2026-10-27T23:21:38Z","spawned_at":"2026-10-27T23:21:38Z","refresh":null,"last_log":"2026-10-28 00:21:38 pid 47627 refresh done in 105ms","installs":[{"name":"default","root":"/Users/me/Library/Application Support/Cursor","found":true,"scanned_at":"2026-10-27T23:21:38Z","scan_duration_ms":77,"dirty_at":null,"workspaces":73,"chats":1144,"subagents":1049,"stale":{"sources":1,"reasons":["global db changed"]}}]}}]}
```

### `cursor cache scan`

`chatkeep cursor cache scan --json [-n] [--full] [--profile NAME]`

Refreshes chatkeep's own index; it reads Cursor's files and never writes them, so it needs no
`-y`. One result, `tool: "cursor"`.

With `-n`, nothing is read again: `data` is
`{"installs": [{"name", "root", "scanned", "stale_sources", "reasons"}]}`, where `scanned` says
whether the installation was ever scanned, `stale_sources` is int or null, and `reasons` lists
what changed (empty when up to date).

```json
{"ok":true,"command":"cursor cache scan","dry_run":true,"results":[{"tool":"cursor","data":{"installs":[{"name":"default","root":"/Users/me/Library/Application Support/Cursor","scanned":true,"stale_sources":1,"reasons":["global db changed"]}]}}]}
```

Without `-n`: `data` is
`{"full", "duration_ms", "path", "installs": [{"name", "workspaces", "chats", "subagents", "changed_sources", "chats_read", "duration_ms"}]}`.

```json
{"ok":true,"command":"cursor cache scan","dry_run":false,"results":[{"tool":"cursor","data":{"full":false,"duration_ms":86,"path":"/Users/me/.chatkeep/index.db","installs":[{"name":"default","workspaces":73,"chats":1144,"subagents":1049,"changed_sources":1,"chats_read":2,"duration_ms":78}]}}]}
```

### `claude cache stats`

`chatkeep claude cache stats --json`

One result, `tool: "claude"`. `data`: `enabled` (false with `CHATKEEP_NO_INDEX`), `exists`,
`path`, `size_bytes`, and `transcripts` (int), `refreshed_at` (time), `schema` (int), each
`null` when the index is off or not built yet.

```json
{"ok":true,"command":"claude cache stats","dry_run":false,"results":[{"tool":"claude","data":{"enabled":true,"exists":true,"path":"/Users/me/.chatkeep/claude-index.db","size_bytes":602112,"transcripts":851,"refreshed_at":"2026-10-27T23:22:04Z","schema":1}}]}
```

### `claude accounts ls`

`chatkeep claude accounts ls --json`

One result, `tool: "claude"`. `data` is `{"accounts": [...]}`, one row per organization of each
desktop app account:

| Field          | Type           | Meaning                                                                    |
| -------------- | -------------- | -------------------------------------------------------------------------- |
| `account`      | string         | Account id.                                                                |
| `label`        | string         | How chatkeep shows it: its name, else its e-mail address, else the start of its id. |
| `name`         | string or null | The name given with `claude accounts name`.                                |
| `email`        | string or null | Its e-mail address, when a login showed it.                                |
| `organization` | string         | Organization id.                                                           |
| `member`       | string         | `account/organization`, the value sync profiles store and accept.          |
| `chats`        | int            | Chats the app lists for it.                                                |
| `on_disk`      | int            | Of those, chats whose transcript is on this machine.                       |
| `signed_in`    | bool           | The app is signed in with this account.                                    |
| `dir`          | string         | The folder the app keeps the list in.                                      |

```json
{"ok":true,"command":"claude accounts ls","dry_run":false,"results":[{"tool":"claude","data":{"accounts":[{"account":"310c27f1-df40-42ae-b1c7-e12574811802","label":"310c27f1","name":null,"email":null,"organization":"d68212da-94ca-4311-9743-1c72b0f9362b","member":"310c27f1-df40-42ae-b1c7-e12574811802/d68212da-94ca-4311-9743-1c72b0f9362b","chats":79,"on_disk":79,"signed_in":false,"dir":"/Users/me/Library/Application Support/Claude/claude-code-sessions/310c27f1-df40-42ae-b1c7-e12574811802/d68212da-94ca-4311-9743-1c72b0f9362b"}]}}]}
```

### `claude accounts doctor`

`chatkeep claude accounts doctor --json`

One result, `tool: "claude"`. `data`:

| Field            | Type            | Meaning                                                                                       |
| ---------------- | --------------- | --------------------------------------------------------------------------------------------- |
| `desktop_app`    | bool            | The desktop app keeps data on this machine.                                                   |
| `desktop`        | object or null  | The account it is signed in with: `{"account", "label", "name", "email", "organizations"}`.  |
| `session`        | object or null  | The account of the Claude Code session chatkeep runs in: `{"account", "label", "email", "organization", "organization_name"}`. |
| `login`          | object or null  | The last login in `.claude.json`, same fields.                                                |
| `claude_command` | string or null  | The `claude` command on `PATH`.                                                               |
| `stale_caches`   | int             | Entries `claude accounts prune` would remove.                                                 |
| `findings`       | array           | `{"level": "ok" | "warn" | "info" | "hint", "text"}`, what the text output concludes.        |

```json
{"ok":true,"command":"claude accounts doctor","dry_run":false,"results":[{"tool":"claude","data":{"desktop_app":true,"desktop":{"account":"447c80ba-de69-436c-8bb8-3d2ecdaa12e4","label":"me@example.com","name":null,"email":"me@example.com","organizations":["3ab1ac9b-9a6b-4b53-9ca1-4205d57ee19d"]},"session":{"account":"447c80ba-de69-436c-8bb8-3d2ecdaa12e4","label":"me@example.com","email":"me@example.com","organization":"c4c4271d-9b29-47cf-9f31-06e3ae5600b2","organization_name":null},"login":{"account":"447c80ba-de69-436c-8bb8-3d2ecdaa12e4","label":"me@example.com","email":"me@example.com","organization":"c4c4271d-9b29-47cf-9f31-06e3ae5600b2","organization_name":"Example Organization"},"claude_command":"/Users/me/.local/bin/claude","stale_caches":7,"findings":[{"level":"ok","text":"Desktop sessions use the account the desktop app is signed in with."},{"level":"hint","text":"Remove the cache entries of accounts not in use: chatkeep claude accounts prune (7 entries)"}]}}]}
```

### `claude sync profiles`

`chatkeep claude sync profiles --json` (alias `claude sync ls`)

One result, `tool: "claude"`. `data`:

- `profiles`: `[{"name", "accounts": [...]}]`, sorted by name. Each account has `account`,
  `label`, `name`, `email` (as in `claude accounts ls`), `organization` (string or null),
  `member` (the stored `account/organization`), and `signed_in`.
- `auto`: the [background sync status](#claude-sync-auto-status).

```json
{"ok":true,"command":"claude sync profiles","dry_run":false,"results":[{"tool":"claude","data":{"profiles":[{"name":"main","accounts":[{"account":"310c27f1-df40-42ae-b1c7-e12574811802","label":"310c27f1","name":null,"email":null,"organization":"d68212da-94ca-4311-9743-1c72b0f9362b","member":"310c27f1-df40-42ae-b1c7-e12574811802/d68212da-94ca-4311-9743-1c72b0f9362b","signed_in":false}]}],"auto":{"state":"stopped","installed":true,"running":false,"log_path":"/Users/me/.chatkeep/sync-log.jsonl"}}}]}
```

### `claude sync log`

`chatkeep claude sync log --json [--last N]` (default 30)

One result, `tool: "claude"`. `data` is `{"entries": [...]}`, the newest N, oldest first:

| Field     | Type            | Meaning                                                                |
| --------- | --------------- | ---------------------------------------------------------------------- |
| `at`      | time or null    | When (the last time, for a repeated change).                           |
| `profile` | string or null  | The sync profile.                                                      |
| `action`  | string          | `add`, `update`, `remove`, or `note` (a message from the watcher).     |
| `chat`    | string or null  | The chat id.                                                           |
| `title`   | string or null  | The chat title, or the text of a note.                                 |
| `account` | string or null  | The `account/organization` whose list changed.                         |
| `source`  | string          | `manual` for a sync someone ran, `watch` for the watcher.              |
| `changed` | array of string | For an update: what differed, e.g. `title`.                            |
| `times`   | int             | How often this happened in a row.                                      |

```json
{"ok":true,"command":"claude sync log","dry_run":false,"results":[{"tool":"claude","data":{"entries":[{"at":"2026-10-27T23:20:07Z","profile":"main","action":"update","chat":"704b8a53-43db-48be-87e4-41dfb6f00b48","title":"Plan the release","account":"9efbfc68-3574-4c71-94bc-b94c905d05d0/3ab1ac9b-9a6b-4b53-9ca1-4205d57ee19d","source":"watch","changed":["lastActivityAt"],"times":1}]}}]}
```

### `claude sync auto status`

`chatkeep claude sync auto status --json`

One result, `tool: "claude"`. `data`: `state` (`on`: installed and running; `stopped`: installed
but not running; `off`), `installed`, `running`, and `log_path`.

```json
{"ok":true,"command":"claude sync auto status","dry_run":false,"results":[{"tool":"claude","data":{"state":"stopped","installed":true,"running":false,"log_path":"/Users/me/.chatkeep/sync-log.jsonl"}}]}
```

### `--version`

`chatkeep --json --version`

```json
{"ok":true,"command":"version","dry_run":false,"results":[{"tool":null,"data":{"version":"1.2.0"}}]}
```

## Writes

These commands change data and take `-y`. With `--json` each needs `-n` (plan only, change
nothing) or `-y` (run), or fails with `needs_confirmation`:

`mv`, `cp`, `rm`, `split`, `combine`, `export`, `import`, `cursor save`, `cursor rx`,
`cursor cache clear`, `queue rm`, `queue clear`, `queue retry`, `queue execute`,
`claude accounts cp`, `claude accounts prune`, `claude cache clear`, `claude sync` (running a
sync), `claude sync set`, `claude sync add`, `claude sync rm`.

These write without asking and take neither flag, as without `--json`: `claude accounts name`,
`claude sync auto on`, `claude sync auto off`, `queue add`.

A dry run (`-n`) returns the same shape as the real run, with what it would do.

### Report

Every write command's `data` is a report:

```json
{
  "title": "Move",
  "warnings": ["skipped 9602d48d5e4595972fd32e1bb8f58cf0: destination missing /tmp/new"],
  "skipped": ["9602d48d5e4595972fd32e1bb8f58cf0"],
  "applied": [],
  "rewritten_keys": []
}
```

| Field            | Type            | Meaning                                                                              |
| ---------------- | --------------- | ------------------------------------------------------------------------------------ |
| `title`          | string          | What ran, for display: see the list below. Do not branch on it.                      |
| `warnings`       | array of string | Warnings, for people.                                                                |
| `skipped`        | array of string | Items left alone (ids or lines).                                                     |
| `applied`        | array of string | One line per change made (or, in a dry run, planned), for people.                    |
| `rewritten_keys` | array of string | Cursor storage keys that were rewritten.                                             |

Nothing to do is a report with empty `applied` and `skipped`. Titles: Cursor `Move`, `Copy`,
`Split`, `Combine`, `Remove`, `Export`, `Import`, `Save`, `Reindex`, `Cache clear`; Claude Code
`Move (Claude Code)`, `Copy (Claude Code)`, `Split (Claude Code)`, `Combine (Claude Code)`,
`Remove (Claude Code)`, `Export (Claude Code)`, `Import (Claude Code)`, `Accounts copy`,
`Account name`, `Accounts prune`, `Claude Code cache clear`, `Sync (Claude Code)`,
`Sync profile`, `Background sync on`, `Background sync off`; queue `Queue remove`,
`Queue clear`, `Queue retry`, `Queue execute`.

Example (`chatkeep mv -n --json --profile default ~/projects/auth /tmp/new`):

```json
{"ok":true,"command":"mv","dry_run":true,"results":[{"tool":"cursor","data":{"title":"Move","warnings":["skipped 9602d48d5e4595972fd32e1bb8f58cf0: destination missing /tmp/new"],"skipped":["9602d48d5e4595972fd32e1bb8f58cf0"],"applied":[],"rewritten_keys":[]}}]}
```

### Per command

| Command                                   | Result `tool`             | `data`                                     |
| ----------------------------------------- | ------------------------- | ------------------------------------------ |
| `mv`, `cp`, `rm`, `combine`, `export`, `import` | each tool it ran for | Report                                     |
| `split`                                   | each tool it ran for      | Report plus `chats`, see [split](#split)   |
| `cursor save`, `cursor rx`, `cursor cache clear` | `cursor`           | Report                                     |
| `claude accounts cp`, `claude accounts name`, `claude accounts prune`, `claude cache clear`, `claude sync`, `claude sync set`, `claude sync add`, `claude sync rm` | `claude` | Report |
| `claude sync auto on`, `claude sync auto off` | `claude`              | Report plus `auto` (the status afterwards) |
| `queue rm`, `queue clear`, `queue retry`  | `null`                    | Report: one `applied` line per entry, `"<verb> <id> <command>"` |
| `queue execute`                           | `null`                    | Report plus `entries`, see [queue execute](#queue-execute) |

Notes:

- `rm` takes a project path, a Cursor workspace hash, or a chat id; `--json` never opens the
  picker, so the target is required.
- `claude sync` never removes chats from an account with `--json`: chats deleted in another
  account are only removed after a yes at a terminal.
- `queue rm` and `queue retry` fail with `failed` for an unknown id, also in a dry run, as the
  real run would.

### `split`

`chatkeep split --json (-n | -y) SOURCE TARGET... [--assign CHAT=TARGET]... [--move]`

`split` suggests a target for each chat from the files it touched. `--assign CHAT=TARGET` sends a
chat to `TARGET` instead; repeat it for more chats, or for one chat and several targets.
`--assign CHAT=` (empty target) keeps that chat out of the split. `TARGET` must be one of the
given targets; `CHAT` must be a chat of the source. Chats without `--assign` keep the suggestion.

A chat the suggestion places nowhere and that has no `--assign`:

- in a dry run (`-n`) it gets no target, and `warnings` starts with a line that says so;
- in a real run (`-y`) the command fails with `missing_argument`.

So an app runs `split -n --json`, shows `chats`, lets the user change them, and runs
`split -y --json` with one `--assign` per changed or unplaced chat.

`data` is a Report plus `chats`, one per top-level chat of the source, sorted by id:

| Field       | Type            | Meaning                                                        |
| ----------- | --------------- | -------------------------------------------------------------- |
| `id`        | string          | Chat or session id: the `CHAT` of `--assign`.                  |
| `title`     | string or null  | Its title.                                                     |
| `suggested` | array of string | Targets the suggestion picked, absolute; empty for none.       |
| `targets`   | array of string | Targets it goes to in this run, after `--assign`; empty for none. |

```json
{"ok":true,"command":"split","dry_run":true,"results":[{"tool":"cursor","data":{"title":"Split","warnings":["1 chat match no target folder: a real run needs --assign CHAT=TARGET for each, or CHAT= to leave one out"],"skipped":[],"applied":["dry-run create workspace 62b2f48189be365121f05c77dbd2b880 for /Users/me/mono/api","dry-run create workspace 0b441eeef91576ef8005ee48530d7cca for /Users/me/mono/web","dry-run copy 1 chats source-hash -> 62b2f48189be365121f05c77dbd2b880","dry-run copy 1 chats source-hash -> 0b441eeef91576ef8005ee48530d7cca","source-hash"],"rewritten_keys":[],"chats":[{"id":"aaaaaaaa-0000-4000-8000-000000000001","title":"Refactor the API client","suggested":["/Users/me/mono/api"],"targets":["/Users/me/mono/api"]},{"id":"bbbbbbbb-0000-4000-8000-000000000002","title":"Style the login page","suggested":["/Users/me/mono/web"],"targets":["/Users/me/mono/web"]},{"id":"cccccccc-0000-4000-8000-000000000003","title":"Update the README","suggested":[],"targets":[]}]}}]}
```

The `applied` lines of a split are for display; their wording differs per tool.

### `queue execute`

`chatkeep queue execute --json (-n | -y) [--continue-on-error]`

Plans every pending entry, then (without `-n`) runs them in order. One result, `tool: null`.
`data` is a Report (`applied`: entries that ran or, in a dry run, would run; `skipped`: entries
whose plan failed or that failed; `warnings`: plan warnings as `"<id>: <warning>"`) plus
`entries`, one per pending entry:

| Field           | Type            | Meaning                                                                                      |
| --------------- | --------------- | -------------------------------------------------------------------------------------------- |
| `id`            | string          | Entry id.                                                                                    |
| `argv`          | array of string | Its stored command.                                                                          |
| `command`       | string          | `argv` quoted for a shell.                                                                   |
| `plan`          | string          | `ok`, `warnings`, or `skip` (the plan failed; the entry is skipped).                         |
| `plan_warnings` | array of string | Warnings of its plan.                                                                        |
| `plan_error`    | string or null  | Why the plan failed.                                                                         |
| `status`        | string          | `planned` (a dry run, or the run stopped before it), `done`, `failed`, `skipped`, or `blocked` (a tool was still open; it waits again). |
| `error`         | string or null  | Why it failed or was skipped.                                                                |
| `results`       | array           | One Report per tool it ran for, each with an extra `dry_run` bool.                           |

When an entry fails (and `--continue-on-error` is not given), or any entry failed or was
skipped, the envelope is an error (`failed`) whose `results` holds this same data. When a tool is
still open, it is `tool_running` with `queue_args: null`, and `results` holds the data so far.

```json
{"ok":true,"command":"queue execute","dry_run":true,"results":[{"tool":null,"data":{"title":"Queue execute","warnings":[],"skipped":[],"applied":["b970b2f4 combine --move --profile resolved 5254ff40f20eb9f28d0493f27aa9307b e3ca3ba788c27c6259e41499d7f0bcd0 -y"],"rewritten_keys":[],"entries":[{"id":"b970b2f4","argv":["combine","--move","--profile","resolved","5254ff40f20eb9f28d0493f27aa9307b","e3ca3ba788c27c6259e41499d7f0bcd0","-y"],"command":"combine --move --profile resolved 5254ff40f20eb9f28d0493f27aa9307b e3ca3ba788c27c6259e41499d7f0bcd0 -y","plan":"ok","plan_warnings":[],"plan_error":null,"status":"planned","error":null,"results":[]}]}}]}
```

## Errors, examples

```json
{"ok":false,"command":"mv","error":{"kind":"missing_argument","message":"mv needs FROM and TO, or --replace FROM TO","hint":"Pass it on the command line; --json never asks.","queue_args":null},"results":[]}
{"ok":false,"command":"mv","error":{"kind":"needs_confirmation","message":"mv changes data and --json never asks","hint":"Pass -n to see the plan, or -y to run it.","queue_args":null},"results":[]}
{"ok":false,"command":"mv","error":{"kind":"missing_argument","message":"/Users/me/projects/auth exists in profiles default, resolved","hint":"Pass --profile with one of: default, resolved.","queue_args":null},"results":[]}
{"ok":false,"command":null,"error":{"kind":"usage","message":"unrecognized subcommand 'bogus'","hint":null,"queue_args":null},"results":[]}
{"ok":false,"command":"update","error":{"kind":"unsupported_in_json","message":"update has no JSON output","hint":"Run it without --json.","queue_args":null},"results":[]}
```
