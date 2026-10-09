@AGENTS.md

## Claude Code

- Step 4 of the workflow is `/code-review` against `main`, step 5 is `/simplify`. Their sub-agents run on
  the `sonnet` model.
- Sub-agents and parallel sessions never check out or switch branches in a shared checkout; work that
  needs its own branch gets its own worktree.
- Talk to the maintainer in Ukrainian; everything written into the repository stays in English.
- When an APK is waiting to be built, watch the run the push started (`gh run watch`) rather than
  dispatching another one.
