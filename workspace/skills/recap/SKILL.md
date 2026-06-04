---
name: recap
description: Summarize what changed in the working directory since a given ref
user-invocable: true
argument-hint: [base ref, default HEAD~5]
---

Summarize what's changed in the current git repository.

If the user supplied a base ref below, diff against that. Otherwise default to `HEAD~5`.

Steps:
1. Run `git log --oneline <base>..HEAD` to list commits.
2. Run `git diff --stat <base>..HEAD` for a file-level summary.
3. Group the changes by area (e.g. "providers", "tests", "docs") and produce
   3-6 bullet points covering the highlights. Skip churny noise like
   formatting-only diffs.
4. Flag anything that looks risky: new dependencies, config changes,
   migrations, anything that touches auth or secrets.

Base ref: $ARGUMENTS
