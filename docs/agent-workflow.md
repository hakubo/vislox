# Changing the config with your own agent (proposal)

> Status: a proposal, not built yet. It replaces the experimental "Change with AI" box (removed in #27), which
> sent prompts to a webhook and only worked with one particular agent setup.

The idea: vislox does not run or talk to an AI. You run whatever agent you like (Claude Code, Codex, a script,
yourself in an editor) against the config repository. vislox stays what it is good at: showing what changed,
and deploying it safely.

## What works today

Everything the agent needs already exists in a `lox.toml` project. (vislox's own edits go into
`<module>/vislox/`; an agent should leave that directory alone, see [editing.md](editing.md).)

1. Start vislox on the project with an admin token:
   `vislox ~/loxone-config/project --admin-token-file ~/.config/vislox/admin-token`.
2. In a terminal, run your agent in the same project directory. It edits the Lox Logic module and compiles it:
   `lox logic …`, then `lox logic compile`. It should not run `lox config push`: deploying is Apply's job.
3. vislox notices the new `out.Loxone` and shows it as the *Pending* revision, with every added, changed,
   removed and moved block marked on the pages.
4. Review it in vislox. **Apply** deploys it (`lox config push --force`, verify), commits and pushes to git.
   **Discard** throws it away.

The agent can work on a separate git branch or worktree too, as long as vislox is started on that checkout.

## What vislox could add

Roughly in order of value; none of these needs vislox to know about AI.

- **Copy context for the agent.** Replace the old "AI context" chips with a "Copy reference" action on a page or
  block: page title, block name, type and UUID, ready to paste into a prompt. Today's equivalent is the URL
  (`#page=…&sel=…`), which carries UUIDs but no names.
- **Show where the pending change came from.** Next to the pending revision, show the module's uncommitted
  `git diff --stat` and `lox logic diff --summary`, so the review starts from the agent's own summary.
- **Branches as candidates.** In a `lox config init`-style repository, list local branches (or worktrees) that
  hold a compiled `out.Loxone` as pending candidates. Picking one compares it with the deployed base; Apply
  deploys that branch's compile and merges it. This lets several agents propose changes side by side.
- **A commit-message hint file.** If the agent writes `.vislox/summary.md` (git-ignored), Apply uses it as the
  commit body, the way it used the AI prompts before.
- **Headless apply for CI.** A `vislox apply` subcommand (or a documented `curl` to `/api/apply` with the token)
  so an agent pipeline can deploy after a human approves in the UI, without vislox having to call anyone.

Open questions before building any of this:

- Should Apply ever run without a person clicking it (e.g. after an approval elsewhere)?
- Branch candidates: does vislox switch branches itself, or only read `out.Loxone` from them with `git show`?
