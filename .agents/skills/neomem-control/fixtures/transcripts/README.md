Capture fixtures contain recorded envelopes and synthetic edge cases, with no personal sessions or credentials.

- Claude Code 2.1.304 user and assistant entries were recorded against a local fake Messages API in a fresh temporary HOME and CLAUDE_CONFIG_DIR, then scrubbed.
- Codex 0.163.0 user and assistant response_item entries were recorded against a local fake Responses API in a fresh HOME and CODEX_HOME without credentials.
- Pi entries were emitted through the installed 0.73.1 SessionManager into a fresh temporary directory, including a branch operation and compaction entry, then scrubbed. Its session format is v3. No model or authentication was used.

Action, compaction and partial-line cases extend these envelopes synthetically. The pi parentId chain ends at the last persisted entry. The unfinished tail is appended in two writes by each proof. Fake secrets are constructed at runtime.
