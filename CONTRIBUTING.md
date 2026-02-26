# Contributing to Fluxora Contracts

First off, thank you for considering contributing to Fluxora! It's people like you that make open-source software such a great community.

## How to Contribute

### 1. Fork & Clone
1. Fork the repository to your own GitHub account.
2. Clone the project to your local machine.
3. Add the original repository as a remote ("upstream").

### 2. Branch Naming Conventions
Always create a new branch for your work. Do not commit directly to the `main` branch. Please use the following prefixes for your branch names:
* `feature/` - for new features (e.g., `feature/multi-period-attestations`)
* `fix/` - for bug fixes (e.g., `fix/stream-overflow`)
* `docs/` - for documentation updates (e.g., `docs/contributing`)
* `test/` - for adding or updating tests (e.g., `test/cancel-from-paused`)

### 3. Development Guidelines
* **Write Tests:** All new code must include comprehensive unit tests.
* **Maintain Coverage:** We enforce a strict **minimum of 95% test coverage**. PRs that drop coverage below this threshold will not be merged.
* **Run Linters:** Ensure your code is properly formatted and passes all linting checks before opening a PR.
* **Update Documentation:** If you are adding a new feature or changing an API, please update the relevant documentation (and NatSpec comments) alongside your code.

### 4. Opening a Pull Request
1. Push your changes to your fork.
2. Open a Pull Request against the `main` branch of the upstream repository.
3. Ensure your PR title is descriptive and follows conventional commit formatting.
4. Link the PR to the relevant issue(s) it resolves.
5. Wait for a maintainer to review your code. 

## Found a Bug or Have a Feature Request?
If you find a bug or have a suggestion, please open an issue first. Be sure to check out our [Issue Templates](.github/ISSUE_TEMPLATE) (if available) to provide all the necessary context.