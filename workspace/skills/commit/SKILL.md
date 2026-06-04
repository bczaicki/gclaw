---
name: commit
description: Stage changes and create a well-structured git commit
user-invocable: true
argument-hint: [optional message hint]
---

You are creating a git commit in the user's current repository.

Steps:
1. Run `git status` and `git diff` (via shell_exec) to see what's changed.
2. Run `git log -5 --oneline` to match the repository's commit message style.
3. Draft a concise commit message (1-2 sentences) that explains the *why*, not the *what*.
   If the user provided a hint below, incorporate it.
4. Stage the relevant files with `git add <paths>` (avoid `git add -A` to skip secrets).
5. Create the commit with the drafted message.
6. Report the commit SHA and short message back.

Never push, never amend, never use --no-verify.

User hint: $ARGUMENTS
