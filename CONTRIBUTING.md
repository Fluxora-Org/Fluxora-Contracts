# Contributing to Fluxora Contracts

First off, thank you for considering contributing to Fluxora! It's people like you that make open-source software such a great community.

## How to Contribute

### 1. Fork & Clone

1. Fork the repository to your own GitHub account.
2. Clone the project to your local machine.
3. Add the original repository as a remote ("upstream").

### 2. Branch Naming Conventions

Always create a new branch for your work. Do not commit directly to the `main` branch. Please use the following prefixes for your branch names:

- `feature/` - for new features (e.g., `feature/multi-period-attestations`)
- `fix/` - for bug fixes (e.g., `fix/stream-overflow`)
- `docs/` - for documentation updates (e.g., `docs/contributing`)
- `test/` - for adding or updating tests (e.g., `test/cancel-from-paused`)

### 3. Development Guidelines

- **Write Tests:** All new code must include comprehensive unit tests.
- **Maintain Coverage:** We enforce a strict **minimum of 95% test coverage**. PRs that drop coverage below this threshold will not be merged.
- **Snapshot Tests:** All behavior changes must update snapshot tests. See [Snapshot Test Workflow](docs/snapshot-tests.md).
- **Run Linters:** Ensure your code is properly formatted and passes all linting checks before opening a PR.
- **Update Documentation:** If you are adding a new feature or changing an API, please update the relevant documentation (and NatSpec comments) alongside your code.

### 4. Snapshot Test Workflow

When your changes affect contract behavior:

1. **Run tests locally:**

   ```bash
   cargo test -p fluxora_stream
   ```

2. **If snapshot tests fail and changes are intentional:**

   ```bash
   SOROBAN_SNAPSHOT_UPDATE=1 cargo test -p fluxora_stream
   ```

3. **Review snapshot changes:**

   ```bash
   git diff contracts/stream/test_snapshots/
   ```

4. **Commit with clear message:**

   ```bash
   git add contracts/stream/test_snapshots/
   git commit -m "test: update snapshots for [specific change]"
   ```

5. **Document in PR:** Explain why snapshots changed and what behavior changed.

See [Snapshot Test Documentation](docs/snapshot-tests.md) for complete guidance.

### 5. Opening a Pull Request

1. Push your changes to your fork.
2. Open a Pull Request against the `main` branch of the upstream repository.
3. Ensure your PR title is descriptive and follows conventional commit formatting.
4. Link the PR to the relevant issue(s) it resolves.
5. **If snapshots changed:** Use the PR template to document what changed and why.
6. Wait for a maintainer to review your code.

## Testing Requirements

### Unit Tests

- All new functions must have unit tests
- Edge cases must be covered
- Error conditions must be tested

### Snapshot Tests

- All state transitions must have snapshot coverage
- Authorization boundaries must be explicit
- Event emissions must be verified
- See [Snapshot Test Authoring Guide](docs/snapshot-test-authoring-guide.md)

### Coverage

- Minimum 95% code coverage required
- Run coverage report: `cargo tarpaulin --features testutils -p fluxora_stream`

## Documentation Requirements

When contributing, update:

- Code comments for complex logic
- Function documentation for public APIs
- `docs/` files for behavior changes
- `README.md` for user-facing changes
- Snapshot test documentation if test patterns change

## Found a Bug or Have a Feature Request?

If you find a bug or have a suggestion, please open an issue first. Be sure to check out our [Issue Templates](.github/ISSUE_TEMPLATE) (if available) to provide all the necessary context.

## Resources

- [Snapshot Test Documentation](docs/snapshot-tests.md)
- [Snapshot Test Authoring Guide](docs/snapshot-test-authoring-guide.md)
- [Snapshot Workflow Quick Reference](docs/snapshot-workflow-quick-reference.md)
- [Coverage Matrix](docs/snapshot-test-coverage-matrix.md)
- [Audit Documentation](docs/audit.md)
