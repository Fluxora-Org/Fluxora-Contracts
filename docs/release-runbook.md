# Release Runbook

## Mainnet Deployment Approval

A mainnet deployment cannot proceed without approval. The `deploy-mainnet` job in our CI pipeline leverages `trstringer/manual-approval@v1` to enforce this rule within the repository configuration. It requires approval from a named reviewer (e.g. `Fluxora-Org/maintainers`) before executing the deployment to Stellar mainnet. Approvals are auditable via the automatically generated issue in this repository.
