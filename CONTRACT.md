# neomem shared contract

neomem owns semantic memory. External systems keep their own content and store
neomem IDs; neomem stores references back without importing that content.

## IDs and lookup

Entity, fact and intake IDs are permanent ULIDs prefixed `ent_`, `fct_`, and
`int_`. IDs are never reused. A merged entity's old ID and aliases resolve to
the surviving entity. `neomem resolve "Edge app" --json` returns matching
entity IDs; `neomem entity Edge --json` returns its facts, linked entities and
external references. Ambiguous names can return multiple resolution matches;
use the returned ID for subsequent operations.

## Tags

`neomem tags --json` reads the registry. Its starting set is `conversation`,
`agent-session`, `work-session`, `code`, `health`, `sleep`, `money`, `purchase`,
`calendar`, `social`, `school`, `travel`, `place`, `media`, `raw`, `system`.
`neomem tags --new healthcare --json` returns the existing `health` tag when
the judge finds the meanings equivalent. New meanings are registered once.
The extractor uses the same registry and duplicate check. Tags are normalized
to lowercase words separated by hyphens, at most 48 ASCII characters.
`neomem search "query" --tags health --json` filters by tag. Memories tagged
`raw` or `system` remain hidden unless `--include-hidden` is passed.

## Links and external references

Internal link types are `updates`, `extends`, `contradicts`, `part_of`,
`derived_from`, `about`, `possibly_same`, and `corrects`. Entity relations
add open labels. The external link type is `mentions`.

An external reference has the form `ext:<system>/<id>`. The system is a
lowercase ASCII letter followed by lowercase letters, digits or hyphens,
at most 48 characters. The ID is 1–256 ASCII letters, digits, underscores,
hyphens or periods. Neither component may be `.` or `..`.

```
neomem link ext:episodes/epi_123 --to ent_01M56PGSJG83SYHRJXJG7DJ13N --type mentions --json
neomem entity Edge
neomem rebuild --json
```

The target must be an existing entity or fact. Merged entity targets are
canonicalized to the survivor. An item linked to both entities before a merge
is listed once under the survivor. Registering the same reference, target and type
again is idempotent. Entity output groups references by system and lists each
reference to follow. References and their type and target live in `state.db`;
`index.db` contains corresponding link rows and rebuild restores them.
There is no external-content field or content-fetching operation.

## Wire calls

CLI commands relay the same request envelope to the daemon. The local HTTP
API accepts `POST /v1/request` with `Content-Type: application/json` and a protocol envelope `{"protocol":1,"request":<body>}` around each of these bodies:

```json
{"command":"tags","arguments":{"new":null}}
{"command":"tags","arguments":{"new":"healthcare"}}
{"command":"resolve","arguments":{"name":"Edge app"}}
{"command":"entity","arguments":{"name":"Edge"}}
{"command":"link","arguments":{"external":"ext:episodes/epi_123","to":"ent_01M56PGSJG83SYHRJXJG7DJ13N","kind":"mentions"}}
{"command":"rebuild"}
```

Responses use `{"reply":"<type>","body":...}`. Failures use `reply: failed`
with a typed `kind` and human-readable `message`. CLI failures exit nonzero.
The generated `contract/neomem.schema.json` describes the actual request and
response types. A drift test compares it with a freshly generated schema.

Regenerate the schema with `cargo run -p neomem-proto --example schema > contract/neomem.schema.json`.

The default HTTP address is `http://127.0.0.1:47391`; configuration can change
its port. Equivalent calls for listing tags and registering a reference are:

```sh
curl -sS http://127.0.0.1:47391/v1/request -H 'Content-Type: application/json' \
  -d '{"protocol":1,"request":{"command":"tags","arguments":{"new":null}}}'
curl -sS http://127.0.0.1:47391/v1/request -H 'Content-Type: application/json' \
  -d '{"protocol":1,"request":{"command":"link","arguments":{"external":"ext:episodes/epi_123","to":"ent_01M56PGSJG83SYHRJXJG7DJ13N","kind":"mentions"}}}'
```

Schema validation describes the wire shape. The daemon also validates domain
rules, including reference syntax and target existence.
