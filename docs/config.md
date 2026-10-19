# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Magpie routing notices (fork extension)

This fork opts non-OpenAI providers and the Magpie `group/` model namespace
into `X-Magpie-Routing-Notice: v1` (including a local gateway configured under
the `openai` provider name).
A compatible Magpie gateway sends `response.magpie_routing` SSE metadata
when a group member starts answering. The TUI displays the actual model as
`gpt-6.1-sol [via <selected group>]` and adds an informational `Failover: A -> B`
notice when a fallback member answers. The model picker and subsequent
requests still use the selected group; routing does not rewrite configuration.
The actual-model display resets on a new turn, thread, or manual model change.
In `exec --json`, fallback notices go to stderr, not assistant/error JSON items.

The event contains a `routing` object with `from_model`, `to_model`, and a boolean
`fallback`. Model identifiers must be nonempty ASCII identifiers of at most 256
bytes using letters, digits, `/`, `-`, `_`, `.`, or `:`. Invalid metadata is
ignored. This extension is separate from OpenAI safety-related model reroutes
and never generates a safety warning. Older gateways simply ignore the header.
Run the locally built CLI with `--no-daemon` so it does not attach to an older,
unpatched shared server. Existing CLI processes need to be resumed with the
patched executable; replacing files cannot update a running process.

## Lifecycle hooks

Admins can set top-level `allow_managed_hooks_only = true` in
`requirements.toml` to ignore user, project, and session hook configs while
still allowing managed hooks from requirements and managed config layers. This
setting is only supported in `requirements.toml`; putting it in `config.toml`
does not enable managed-hooks-only mode.
