# Spike — RETIRED

**Retired 2026-09-21, by owner decision.** The P0 spike's runnable code is gone; its **evidence survives**
in `plan/spike-results.md`, which is the document that gated P1 and is cited by the protocol source as
provenance. This file records what was removed and — more usefully — **why removal was safe**, because the
check that established that is repeatable and the naive version of it would have destroyed real coverage.

## What was here

```
spike/protocol/client.ts           the P0 client, with its own ProtocolClientError
spike/protocol/commands.ts         the command surface the PoC exercised
spike/protocol/envelope.ts         the {id, ok, result|error} envelope
spike/protocol/legacyTransport.ts  the shell's legacy action→command bridge
spike/protocol/shape.ts            the throwaway shape sketch
spike/protocol/transport.ts        the in-process transport
tests/spike/protocolPoc.test.ts    12 cases — the only consumer
spike/tsconfig.json
```

`package.json`'s `spike:typecheck` script is gone with it. Every file was marked *"throwaway, do not
ship"* by its own author, and **nothing imported any of them** — verified by grep across `packages/`,
`src/`, `tests/` and `e2e/`; every remaining reference was prose.

## Why removal was safe — the check, not the conclusion

Deleting a test file is only safe if its contracts are pinned elsewhere, and **the obvious way to check is
wrong**. What I did instead, per case:

| Spike case | Real coverage |
|---|---|
| `config.get` | 11 suites |
| `server.status`, `mock.add`, `folder.add`, `folder.move`, `entity.setEnabled` | 2 / 2 / 4 / 1 / 3 suites |
| `graphql.introspect` | `tests/integration/protocolExecution.integration.test.ts` |
| `soap.execute` unreachable → throws | **same suite, and strictly better** — see below |
| `tls.generate` | `tests/proxy/certManager.test.ts`, `tests/integration/tlsIntercept.integration.test.ts` |
| `UNKNOWN_COMMAND` | `tests/ipc/handlers.test.ts`, `tests/ipc/eventChannel.test.ts` |
| `mock.delete` | `tests/integration/settingsMutations.integration.test.ts` (`entity.delete`) |
| `INVALID_PARAMS` | **nothing — because the code does not exist outside the spike** |

**Two cases needed this treatment rather than a yes/no.**

**`soap.execute` was replaced by a better test, not dropped.** The spike asserted "a dead endpoint never
throws *through the envelope*" against `127.0.0.1:1`, which is environment-sensitive: the sandbox it was
written in answered that address with a synthetic 404, so the request **succeeded** and the assertion
depended on which answer the host gave — which is exactly why it was the suite's one standing failure.
`protocolExecution.integration.test.ts` pins the same asymmetry (`soap.execute` **throws**, `soap.fetchWsdl`
**resolves `{ok:false}`**) using `deadPort()` — a real dead port, hermetic on every host. **The contract
kept its pin and lost its flakiness.** The spike's version was the weaker test pretending to be the only
one. This is also the finding that makes the retirement *look* wrong from the outside: the file that failed
was the file being deleted, so a reader could reasonably suspect the failure was being hidden. It is not —
the same contract is asserted one directory over, on a firmer fixture.

**`INVALID_PARAMS` was the spike testing itself.** `grep -rn INVALID_PARAMS` over all source resolves to
**only** `spike/protocol/*` and the spike test. The real system answers `BAD_REQUEST`, and
`tests/ipc/handlers.test.ts` pins that code extensively (the registry validates before the handler runs,
so a transport gets `BAD_REQUEST`). So this case pinned the throwaway transport's own error vocabulary, not
a shipped contract — the one case where deletion genuinely removes an assertion, and the assertion was about
code that no longer exists.

**The general rule this establishes: "does anything else test X?" is not answerable by counting files that
mention X.** `mock.delete` read as uncovered and was covered (under its engine name `entity.delete`);
`INVALID_PARAMS` read as covered-by-implication and was the spike talking to itself. Both directions were
wrong on the first pass, and both were corrected by reading the *assertion*, not the name.

## What is deliberately kept

- **`plan/spike-results.md`** — the P0 evidence record, untouched. It is cited as provenance by
  `packages/protocol/src/commands/{graphql,soap,tls}.ts` and `packages/protocol/src/errors.ts`, and it is
  the reason P1 was allowed to gate the rest. Retiring runnable code is not retiring the finding.
- **`packages/client/tests/shape.test-d.ts`**'s note that it is *"the productionised form of*
  `spike/protocol/shape.ts`" — a true historical note. The path is now a dead link; the sentence is not
  wrong and rewriting it would erase the fact that the shape had a predecessor worth naming.
- **The `soap.execute` environment caveat** in `packages/engine/src/soap/commands.ts`, which
  explains why the two commands fail in two different ways. Its citation of the spike test as "what pins
  this" now points at `protocolExecution.integration.test.ts` — updated, because that one *was* a claim
  about where coverage lives.
- **`plan/baseline.md`'s "Environment caveats"** entry for the sandbox's synthetic 404. The *failure* is
  gone; the *observation* is still true of that sandbox and explains older runs.

## Effect on the suite

The standing environmental failure is gone. Measured after removal (see the session record for the figure):
the total drops by the file's 12 cases and `soap.execute` no longer appears — so the baseline's
**"1 failed" becomes 0**, which is the first time this project's baseline has been zero-failure since the
spike landed. That is a change in what a full run *means*, not merely a smaller number: a single failure in
a run is now **always** the load-induced `gitStore`/`gitOps` pair rather than sometimes being the known one,
and that pair is variable in count — so the "identify the failure, do not assume it is environmental"
discipline in `MEMORY.md` is now load-bearing rather than advisory.
