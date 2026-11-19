# Decision records

One file per decision or deliberate deferral. Filenames are descriptive kebab-case slugs with no numeric ids; search with `ls` and `grep -l`.

Each file starts with a front-matter block:

```
---
status: open | decided | superseded
date: YYYY-MM-DD
revisit-when: <trigger or none>
supersedes: <filename>   (optional)
---
```

followed by the sections `Question`, `Chosen`, `Alternatives`, `Consequences`.

Placement rule: a choice made or consciously postponed with a revisit trigger goes here; work or investigation still to do goes in `docs/todo/`. `cargo xtask decisions` lists open decisions and every todo.
