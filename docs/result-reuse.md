# Experimental exact result reuse

Enable `ga4gh-lib`'s optional `workflow-contracts` feature and use
`workflow::reuse`. The module lets a consumer replace one prepared deterministic
TES computation with results of an original execution at another provider. It
never submits a task and never creates a replacement TES/WES ID. Within one
engine and shared cache, prefer its native reuse mechanism, such as
[Nextflow](https://docs.seqera.io/nextflow/cache-and-resume/),
[Cromwell](https://cromwell.readthedocs.io/en/latest/cromwell_features/CallCaching/)
or [DNAnexus](https://documentation.dnanexus.com/user/running-apps-and-workflows/job-reuse).

## Author contract and identity

The recipe author explicitly opts in with a named, revisioned determinism
contract. Network-dependent, secret-dependent, undeclared or stochastic behavior
is ineligible. A declaration is an author assertion; this SDK does not sandbox
arbitrary programs or prove their reproducibility. A comparison policy or
near-equal scientific metrics never authorizes this exact-reuse policy.

Version 1 fingerprints the complete `RecipeIdentity` using SHA-256 over compact
JSON with recursively sorted object keys. Array order and typed values remain
significant, including the serialized distinction between JSON `-0.0` and `0.0`.
Eligibility compares canonical fingerprints rather than JSON value equality.
The format is versioned; it does not claim interoperability with
engine-specific cache hashes. Identity includes:

| Component | Required evidence |
| --- | --- |
| Executable | SHA-256 of executable/script bytes and immutable OCI image `name@sha256:…` |
| Invocation | Exact command, relevant environment, configuration and explicitly typed parameters |
| Inputs | Content SHA-256, byte size and relative logical input path |
| References | Named version and content SHA-256 for each reference |
| Outputs | Exact I/O-profile schema-byte digest and artifact-pointer → relative-path contract |
| Compatibility | Exact OS, architecture, runtime and any additional required runtime properties |
| Existing invocation | Optional phase-two prepared invocation fingerprint |
| Reproducibility | Author, contract revision, explicit opt-in and dependency declarations |

The caller builds the requested identity from its **current prepared task**, not
by copying identity from an imported receipt. It is responsible for hashing the
actual intended input and executable bytes and declaring every behavior that
matters. Provider-specific staging roots, signed access URLs and service/run IDs
are outside recipe identity. Logical paths and output layout are inside it.
Changed content, parameters, executable, command, environment, configuration,
reference version/digest, output schema/layout or required architecture changes
identity and prevents reuse. Paths must be relative and cannot contain traversal,
empty segments, backslashes or drive prefixes.

## Explicit receipt exchange and current checks

`ReuseReceipt` embeds the existing phase-two `results::ResultRecord`. Its small
eligibility envelope adds recipe identity, original service/run identity, the
exact result run reference, a provenance-source label, and artifact relative
paths, stable references, sizes and SHA-256 values. Existing typed values,
schema annotations, execution state,
contract diagnostics and provenance context remain available. CrateGen remains
the RO-Crate conversion/provenance component. There is no new artifact package,
global cache index, upload service, DRS checksum-search requirement or proposed
TES/WES field.

Export with `receipt.export()` into a user-controlled workspace and import with
`ReuseReceipt::import(bytes)`. Bound file reads before importing untrusted data.
Durable references reject userinfo, queries and fragments. Do not put credentials
into parameters, environment, source labels or result context; these are durable
caller-owned scientific metadata. Use ephemeral access plans for credentials.

For each assessment/action, independently establish current provenance trust:

- `ReceiptTrust::Pinned` binds the canonical receipt content digest and original
  service to an externally trusted, authorized source and a nonempty evidence
  label. Obtain that pin from a trusted producer/workspace or an external
  attestation process. Hashing an untrusted imported receipt and trusting that
  hash alone proves neither origin nor correct execution.
- `Untrusted` rejects reuse. `Unknown` produces `UNVERIFIED`.
- Re-evaluate source policy on every action; previously accepted source trust
  does not override revocation. The prototype supplies no public-key signing,
  federated identity, catalogue discovery or automatic trust policy.

`assess` returns `ELIGIBLE`, `INELIGIBLE` or `UNVERIFIED` with machine-readable
reason codes. It requires COMPLETE execution, a matching pinned output contract,
conformant typed outputs, consistent artifact manifests and a matching complete
recipe. The origin's durable run reference must match the one in the typed result.
Imported artifact verification evidence is historical and is cleared
before current verification.

A caller-owned access adapter receives each `ArtifactReference` and must perform
a **fresh authorized read for the current principal**. Return `ArtifactRead` with
bytes and measured artifact payload bytes transferred, or return
`AccessFailure::{Denied,Missing,Unverified}`. Revoked access, missing bytes, size
mismatch or digest mismatch reject reuse; uncertain availability is UNVERIFIED.
A local read reports zero transferred bytes. Enforce transport/time/body limits
before allocating an artifact; `max_bytes` additionally bounds manifest totals
and bytes accepted by this module. Never return stale cached bytes as evidence
of current access. Credentials and raw access errors are not stored in reports.

Use existing DRS resolution/access planning from #134/#135 to negotiate fresh
capabilities, and the `ResultRecord` verifier from #144 where appropriate. The
reuse check calls its existing `verify_bytes` with the externally trusted
manifest checksum and records fresh evidence; it does not infer trust from a
newly computed checksum. The demonstration also composes
`ResultRecord::verify_access_plan` before receipt export.

```rust,ignore
let receipt = ReuseReceipt::import(&authorized_workspace_bytes)?;
let decision = receipt.assess(&current_task_identity, &current_trust, max_bytes,
                             read_current_artifact);
// An assessment is informational. The explicit action checks everything again.
let reused = receipt.reuse_for(provider_b_api_root, &current_task_identity,
                              &current_trust, max_bytes, read_current_artifact)?;
// reused.original.service and reused.original.run_id identify provider A.
// reused.result.run_reference is the original reference; no provider B job exists.
```

`ReusedResultRef` contains the original execution, the service for which the
computation was prepared, trusted receipt/recipe digests, original typed result,
artifact manifest and measurements. It cannot be deserialized into a substitute
for performing current checks. This is a checked reference at the time of the
read, not a lease guaranteeing future availability. Consumers that materialize
files must preserve the relative paths and independently control their destination.
`prepared_for_service` is a caller-supplied label, not evidence that the second
provider accepted or ran a task.

## Reproducible local experiment

Run from the repository root:

```sh
cargo test -p ga4gh-lib --features workflow-contracts --test result_reuse --locked
python3 examples/result-reuse/run.py
```

Docker must already have the pinned Python image in the selected Docker context.
The adapter uses `--pull=never` and never downloads or deletes unrelated images.
It uses Docker context `default` unless `GA4GH_REUSE_DOCKER_CONTEXT` names another
trusted local context; it does not change the global Docker context. Check
`docker --context default info` before the run. On a Docker Desktop setup where
`default` has no daemon, select its local context explicitly, for example
`GA4GH_REUSE_DOCKER_CONTEXT=desktop-linux python3 examples/result-reuse/run.py`.
On macOS, workspaces default below this checkout's ignored
`target/result-reuse` directory. Use `--work` if the selected Docker daemon
cannot bind-mount that path. For a clean Linux CI runner:

```sh
docker pull python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91
GA4GH_REUSE_DOCKER_CONTEXT=default python3 examples/result-reuse/run.py --work /tmp/result-reuse
```

Two cooperating loopback TES adapters expose separate submission counters. The
SDK submits the allowlisted single-container recipe once to A. Its author-defined
contract processes the public synthetic FASTA in `examples/result-reuse/reference.fa`
into an ASCII-sorted k-mer histogram, with k=3, no network/secrets/randomness, pinned
script/image, exact CPython 3.13, Linux and the observed arm64 or x86_64 architecture.
The fixture is invented public data with no external data dependency. A known
expected histogram independently checks computation output. Docker enforces no
network, read-only root, one CPU, 128 MiB and a bounded process count. The worker
accepts only this known recipe; submitted URLs and commands are never executed
as arbitrary instructions.

The consumer exports/imports the receipt, authorizes that exact workspace export,
prepares B's computation and explicitly substitutes the original result. B's task
list and counters prove that no B job was submitted. Separate cases change k,
revoke access, corrupt same-length bytes, delete bytes and reject provenance.
The runner stops both adapters and leaves `receipt.json`, `evidence.json` and
container diagnostics in its workspace. These local HTTP references are fixtures
whose availability ends when the adapters stop; production references need the
owning provider's retention and authorization policy.

Observed Mac experiment: A accepted/executed **1** container task; B accepted and
executed **0**. One prepared B computation was replaced. The reuse action read,
revalidated and transferred **114 artifact payload bytes**, independently matching
A's bytes-sent counter delta. Initial verification read another 114 bytes; the
corruption rejection transferred another 114 bytes, for **342 bytes total** across
these checks. Denied/missing artifacts and metadata/trust rejections transferred
no artifact payload. No CPU-time, monetary savings or future-hit rate is inferred.
Evidence is real local container execution through cooperating TES adapters; it
is not a live cloud-provider, federated identity or full TES conformance result.
