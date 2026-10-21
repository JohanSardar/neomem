# neomem-pi

Install with `pi install npm:neomem-pi`, or load `index.js` with `pi --extension`.
Enable automatic recall with `neomem recall on --agent pi` or `neomem recall on --project /absolute/project`.
The extension uses `NEOMEM_HOME` (default `$HOME/.neomem`) and reads only its `config.toml`.

The extension registers neomem's read tools for the model: `neomem_search`, `neomem_entity`, `neomem_resolve`, `neomem_show`, `neomem_history`, `neomem_raw` and `neomem_useful`. pi has no MCP, so these take the place of the MCP tools Claude Code and Codex get.

`before_agent_start` returns approved memories as a hidden custom context message, with fact IDs and a 10,000-character cap. Capture signals on `turn_end`, `agent_end`, and `session_shutdown` return synchronously. Requests share one framed Unix socket connection; recall fails open after 3.5 seconds.

`neomemctl prove recall.hooks` runs the installed pi package headlessly with temporary HOME, config, sessions, and a fake OpenAI-compatible provider. It checks the recorded model request for injected IDs and the daemon intake for the completed turn. No real agent configuration or credentials are used.
