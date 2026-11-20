# Rules ledger format

File: docs/rules-ledger.md. One line per rule or fact added through a retro. The ledger tracks rules; it never holds their text — the text lives in the sink it was routed to.

    <date> | <rule or fact, shortened> | <sink: file> | <evidence: plan and friction entry> | <expected effect> | <status>

status: active | automated | retired

- A lesson that is a decision — a choice with a rejected alternative, or a gate baseline raise — also gets a `docs/decisions/` record per the Decision log convention; the ledger row links it.
- Review each active rule against the last N friction logs and `cargo xtask decisions`.
- Mark a rule automated, and propose deleting its prose, once `cargo xtask check` enforces it.
- Propose retiring a rule whose friction did not recur and whose cost in context outweighs it.
- Propose reverting a rule whose expected effect did not materialise; supersede the linked decision record the same way.
