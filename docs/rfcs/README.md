# Provider contract proposals

These RFCs describe proposed provider extensions. They do not describe
implemented GA4GH TES endpoints or currently supported SDK features.

| Proposal | Scope | Feasibility example | Tracking issue |
| --- | --- | --- | --- |
| [Leased interactive sessions](leased-sessions.md) | Ownership, reconnect, lease renewal and cleanup for a live service | [Local session provider](../../examples/provider-contracts/session_feasibility.py) | [Ecosystem #99](https://github.com/elixir-cloud-aai/generic-issues/issues/99) |
| [Workload-bound secrets](workload-secrets.md) | Brokered secret access bound to requester, execution intent and workload identity | [OpenBao JWT and KV-v2](../../examples/provider-contracts/openbao_feasibility.py) | [Ecosystem #100](https://github.com/elixir-cloud-aai/generic-issues/issues/100) |

Each RFC states the current evidence and the remaining requirements for an
operator-backed implementation. The [example guide](../../examples/provider-contracts/README.md)
describes setup and limitations. The examples do not implement a TESK deployment.
