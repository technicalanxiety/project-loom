# Project Stance

Loom is personal memory infrastructure. I am building it for my own use.

It is published under MIT because some readers may find it useful as-is or as a starting point for their own build. I will not be reviewing pull requests, answering issues, or supporting your deployment. Fork it freely. The architecture and the documentation are yours to take.

## What I commit to

- Keeping the main branch working on my machine. If it breaks for me, I fix it. If it breaks only for you, that is your fork's problem to solve.
- Writing publicly about what I learn using it, at [technicalanxiety.com](https://www.technicalanxiety.com).
- Not silently rewriting git history in ways that break forks downstream. Branch hygiene matters even for personal projects.
- Honest documentation. If something does not work, or works with caveats, the docs say so.

## What I do not commit to

- A release schedule. Versions ship when I am ready for them.
- Backwards compatibility across major versions. Schema migrations will be documented, but preserving your fork's data is your responsibility.
- Feature requests. If you want something Loom does not do, fork and build it.
- Bug reports that I cannot reproduce in my own usage. If it happens on my machine, it gets fixed. If it happens only on yours, that is a fork-level concern.
- Support, installation help, or guidance on adapting Loom to use cases I do not share.

## What this means for the scope of the project

Loom solves a problem I have. The problem is cross-tool AI memory scatter for someone who does serious work across Claude, ChatGPT, Copilot, Claude Code, and whatever else shows up next. The architecture reflects that audience of one: local-first, Postgres-native, MCP-integrated where MCP exists, bootstrap-driven for everything else.

Design decisions that optimize for other audiences (enterprise compliance, non-technical users, SaaS deployment, multi-tenant architecture) are not being made. If your needs diverge from mine, the fork is the mechanism by which your version of Loom reflects your needs. That is a feature of the license, not a bug in my maintainership.

## What this means if you want to use it anyway

The install path is documented. It assumes you can run Docker Compose, edit configuration files, and read error messages. The operational path is documented. It assumes you care enough about your own data to back it up. The mental model is documented. It assumes you will read it before you start using the tool, because misuse is often a function of wrong mental model, not wrong button press.

If that matches what you want, welcome. If what you want is a maintained product with a roadmap and a support channel, this is not it, and that is fine. There are other projects in this space. Some of them are commercial, with the accountability and the constraints that commercial products bring. Loom is not one of them.
