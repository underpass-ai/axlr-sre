# opsgenie-mcp and opsgenie-mirror

Opsgenie for a triage bot, through its REST API with an API key, within a scope of teams and
escalations. Two binaries over the same use cases:

| binary            | what it does                                                                                   |
|-------------------|------------------------------------------------------------------------------------------------|
| `opsgenie-mcp`    | MCP server over stdio: `doctor`, `discover`, `alerts_open`, `alert`, `alert_notes`; with `OPSGENIE_WRITES=true`, `alert_acknowledge`, `alert_note`, `alert_close` |
| `opsgenie-mirror` | daemon: a new alert of the scope opens an axlr-bots incident; when the alert closes, the incident is resolved |

Alerts outside the scope are neither shown nor changed, by either binary.

```text
bot ──▶ opsgenie-mcp ──GenieKey──▶ api.opsgenie.com | api.eu.opsgenie.com
                                       /v2/alerts, /v2/teams, /v2/escalations

Opsgenie ──poll──▶ opsgenie-mirror ──axlr-bots incident open|resolve──▶ control plane
```

## Install and register

```bash
make -C tools install        # builds in release and copies both binaries to ~/.local/bin
```

```json
{
  "command": "/home/<user>/.local/bin/opsgenie-mcp",
  "env": {
    "OPSGENIE_API_KEY": "<API integration key>",
    "OPSGENIE_TEAMS": "Platform",
    "OPSGENIE_ESCALATIONS": "Critical"
  }
}
```

## Configuration

Everything comes from the environment. The key is read once at start and never written: not to
a file, not to the log, not into any answer or error.

| variable               | required     | meaning                                                                                     |
|------------------------|--------------|---------------------------------------------------------------------------------------------|
| `OPSGENIE_API_KEY`     | yes          | an API integration key (`GenieKey`); read access for the reads, create/update for the writes |
| `OPSGENIE_URL`         | no           | `https://api.opsgenie.com` when unset; `https://api.eu.opsgenie.com` for an EU account. `http` only for localhost |
| `OPSGENIE_TEAMS`       | one of the two | comma-separated team names or ids                                                         |
| `OPSGENIE_ESCALATIONS` | one of the two | comma-separated escalation names or ids                                                   |
| `OPSGENIE_WRITES`      | no           | `true` publishes the three writes; off when unset                                           |
| `OPSGENIE_USER`        | no           | the user the writes are signed as in Opsgenie, 100 characters at most                       |

**Scope.** An alert is in when one of its responders is a configured team (`type: team`) or
escalation (`type: escalation`). The listing carries responders as ids; names are resolved
through `/v2/teams` and `/v2/escalations`, which need a key that can read them. With ids only,
no listing is needed. A name Opsgenie does not have is an error, never a silently smaller scope.
An empty scope is refused: it would be every alert of the account.

For an EU account the default host answers 401; `doctor` says so and names `OPSGENIE_URL`.

## Tools

Answers are JSON. A failure is `isError` with `error` and, when there is one, `hint`. Arguments
are validated before any request.

| tool                | arguments                                 | answer                                                                                         |
|---------------------|-------------------------------------------|------------------------------------------------------------------------------------------------|
| `doctor`            | —                                         | the configuration without the key, whether the key reads alerts, the scope resolved to ids, writes on or off; an action for what fails |
| `discover`          | —                                         | the scope with ids and names, how many alerts are open in it by priority, how many outside, which tools read and which write |
| `alerts_open`       | `limit` 1–100 (20), `offset`              | the open alerts of the scope, newest first: `tinyId`, `id`, `message` (300 chars), `priority`, `acknowledged`, `count`, `created`, `via`; `total`, `excluded`, `next_offset`, `complete` |
| `alert`             | `alert` (tiny id, `#123`, or id)          | one alert with description (2 000 chars), details (50 keys), tags, source, responders with names |
| `alert_notes`       | `alert`, `limit` 1–100 (20)               | its notes, newest first, 2 000 characters each                                                 |
| `alert_acknowledge` | `alert`, `note`?, `confirm`               | preview, or done with `requestId` and `processed`                                              |
| `alert_note`        | `alert`, `note`, `confirm`                | the same                                                                                       |
| `alert_close`       | `alert`, `note`, `confirm`                | the same; the note travels in the close call, so no close lands without its reason             |

**Writes.** Published only with `OPSGENIE_WRITES=true`; otherwise a call to one is refused with
the way to enable them. Each previews unless `confirm=true`: the preview reads the open alerts,
shows the one it would change and sends nothing (tested with doubles). The alert is read by
its reference, one GET, never through the listing. Only an open alert of the scope can be
changed; a closed one or a foreign one is refused by name. An acknowledge of an alert already
acknowledged says `already: true` and, confirmed, is still sent. Opsgenie takes
writes asynchronously: after the `202`, the request's status is read once, and `processed` is
`verified`, `pending` (read the alert again in a moment), `failed` (with Opsgenie's reason) or
`unknown`.

**Bounds.** The open alerts are read in pages of 100, 20 pages at most, oldest first so new
alerts append while the pages are walked; past that `complete` is false. Every text in an answer
is cut at a stated length.

## The mirror

```bash
opsgenie-mirror [--once] [--dry-run]
```

| variable                   | required | meaning                                                                 |
|----------------------------|----------|-------------------------------------------------------------------------|
| the `OPSGENIE_*` above     | yes      | the same Opsgenie and the same scope                                    |
| `OPSGENIE_MIRROR_TENANT`   | yes      | the axlr-bots tenant the incidents belong to                            |
| `OPSGENIE_MIRROR_BOT`      | yes      | the triage bot that takes them                                          |
| `AXLR_BOTS`                | no       | the `axlr-bots` executable; `axlr-bots` on the PATH when unset          |
| `OPSGENIE_MIRROR_INTERVAL` | no       | seconds between ticks, 15 to 3 600; 60 when unset                       |
| `OPSGENIE_MIRROR_STATE`    | no       | the state file; `$XDG_STATE_HOME/opsgenie-mirror/state.json` or `~/.local/state/opsgenie-mirror/state.json` |

`axlr-bots` finds its control plane the way it always does (`AXLR_BOTS_SOCKET`, `AXLR_BOTS_STATE`
in the environment): the mirror passes nothing of its own.

Every tick reads the open alerts of the scope:

- **First tick** (no state): a baseline. The open alerts are noted as seen and nothing is opened.
- **New alert**: `axlr-bots incident open --tenant <t> --bot <b> --source opsgenie --severity
  <sev> --group-key opsgenie:<alert id> --fingerprints <tinyId> --title "<message> (Opsgenie
  #<tinyId>)" --objective "<tinyId, priority, created, scope entry, message, source, occurrences,
  tags>"`. P1 is sev1, P2 sev2, P3 sev3, P4 and P5 sev4. The incident id comes from
  `incident.id` of the JSON answer; a repeat of the same group key is fine (`created: false`).
- **Alert no longer open**: `axlr-bots incident resolve <id> --reason "closed in Opsgenie"`. An
  incident already resolved counts as done. A baseline alert is only forgotten.
- An incomplete listing (more than 20 pages) resolves nothing that tick.
- A failure with one alert is logged and retried next tick; a failure reading Opsgenie or the
  state aborts the tick with nothing touched. The loop goes on. Ctrl-C or SIGTERM stop it.
- `--once`: one tick, exit 1 on any failure. `--dry-run`: one tick on stdout saying what it
  would open or resolve; it calls nothing and saves nothing.
- Log on stderr (stdout in dry run), UTC stamps, lines only when something happened or failed.
  Never the key.

State: directory 0700, file 0600, written through a temporary file and `rename`:

```json
{"version": 1, "baseline_done": true, "seen": {"<alert id>": "<incident id>", "<baseline alert id>": null}}
```

A corrupt file is an error with its path, not a fresh start: delete it and the mirror begins
again with a baseline.

The mirror runs one way: Opsgenie into axlr-bots. The way back (an incident resolved in
axlr-bots closing its alert) is phase 3 of the design.

## Layout

```text
src/
├── bin/opsgenie_mcp.rs, opsgenie_mirror.rs   composition
├── domain/        ServerConfig, Scope, Directory, ResolvedScope, Alert, AlertRef, Note,
│                  Request, MirrorConfig, MirrorState, MirrorPlan, IncidentDraft
├── ports/         OpsgenieClient, IncidentSink, StateStore
├── application/   ReadAlerts, InspectAlerts, ChangeAlerts, DiagnoseSetup, DiscoverScope,
│                  MirrorIncidents
└── adapters/
    ├── inbound/   tool catalogue, OpsgenieMcpHost, MirrorDaemon
    └── outbound/  HttpOpsgenieClient (reqwest), AxlrBotsCli (subprocess), JsonStateFile
```

## Verified

Against a fake Opsgenie on localhost and a fake `axlr-bots` (shell), 3 November 2026: every
tool over stdio, preview then confirmed close and acknowledge, the refusals of a foreign and of a
closed alert; the mirror's dry run, baseline, open, resolve, an unreachable Opsgenie and a corrupt
state. Not exercised against a real Opsgenie account: the shapes of `/v2/alerts`,
`/v2/alerts/{id}`, `/v2/teams`, `/v2/escalations`, the notes, the `202` answers and the request
status come from the API reference and are pinned by wiremock tests. `axlr-bots incident
open|resolve` comes from the control plane's own CLI and tests.
