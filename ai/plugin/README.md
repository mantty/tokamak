# Tokamak AI plugin

This skill-only plugin helps Codex and Claude Code build, run, debug, and
design Tokamak applications.

## Install from this repository

In Codex, add the repository marketplace:

```sh
codex plugin marketplace add mantty/tokamak --ref main
codex plugin add tokamak@tokamak
```

The first command adds the repository marketplace; the second installs the
plugin from it. You can also install it from the Plugins Directory after
adding the marketplace.

In Claude Code, run:

```sh
claude plugin marketplace add mantty/tokamak
claude plugin install tokamak@tokamak
```

The equivalent interactive commands are `/plugin marketplace add
mantty/tokamak` and `/plugin install tokamak@tokamak`.

The skill is available as `tokamak-development` in Codex and as
`/tokamak:tokamak-development` in Claude Code.

## Test a checkout directly

```sh
claude --plugin-dir ./ai/plugin
```

For Codex, open the checkout as a trusted repository. Its
`.agents/plugins/marketplace.json` is discovered as a repo marketplace.

After changing the skill, start a new session or reload the installed plugin.
