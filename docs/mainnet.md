# Mainnet Deployment Checklist

**Alignment Verification**: See [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) for complete mapping between deployment procedures and protocol semantics.

## Externally Visible Assurances

This checklist ensures deployment correctness through:

1. **On-chain verification**: All deployment steps produce observable on-chain state
2. **Protocol alignment**: Deployment procedures match documented protocol semantics
3. **Authorization boundaries**: All roles and permissions are explicitly verified
4. **Time and numeric boundaries**: All edge cases are tested post-deployment

Treasury operators, recipient applications, and auditors can verify deployment correctness using only on-chain observables and published documentation.

## Risk Summary and Security Considerations

### Critical Risks

**Immutable Contract**: Once deployed to mainnet, the contract cannot be modified or upgraded. All bugs, vulnerabilities, or design issues will be permanent. Thorough testing and auditing are essential before deployment.

**Admin Key Security**: The admin key has privileged access to pause streams and perform administrative functions. Compromise of this key could result in:

- Unauthorized pausing of active streams
- Disruption of service for users
- Loss of trust and reputation

**Token Address Verification**: Incorrect token addresses in the contract configuration will result in:

- Funds being locked or sent to wrong addresses
- Inability to process withdrawals correctly
- Permanent loss of user funds

### Security Best Practices

- **Audit Requirement**: A professional security audit by a reputable firm is strongly recommended before mainnet deployment
- **Key Management**: Use hardware wallets or secure key management systems for admin keys. Never commit keys to the repository
- **Multi-signature**: Consider using multi-sig wallets for admin functions to prevent single points of failure
- **Address Verification**: Triple-check all token addresses and contract parameters before initialization
- **Testnet Validation**: Deploy and test extensively on testnet with the exact same configuration planned for mainnet

### Accountability

The deploying operator is responsible for:

- Verifying all contract parameters and addresses
- Securing admin keys and access controls
- Ensuring adequate testing and audit coverage
- Communicating risks to stakeholders
- Maintaining operational security post-deployment

## Deployment Checklist

### 1. Build

- [ ] Ensure all dependencies are up to date
- [ ] Build contract with release optimizations
- [ ] Verify build reproducibility
- [ ] Generate and save contract hash

```bash
cargo build --release --target wasm32-unknown-unknown
```

### 2. Pre-Deployment Verification

- [ ] All tests passing (unit, integration, edge cases)
- [ ] Security audit completed and all issues resolved
- [ ] Code review completed by multiple team members
- [ ] Testnet deployment successful with identical configuration
- [ ] Admin key secured (hardware wallet or secure key management)
- [ ] Token addresses verified and documented
- [ ] Deployment parameters documented and reviewed

### 3. Deploy

- [ ] Connect to mainnet RPC endpoint
- [ ] Verify sufficient balance for deployment fees
- [ ] Deploy contract to mainnet
- [ ] Record deployment transaction hash
- [ ] Record deployed contract address
- [ ] Backup deployment artifacts

```bash
# Example deployment command (adjust for your environment)
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/stream.wasm \
  --source <DEPLOYER_SECRET_KEY> \
  --network mainnet
```

**WARNING**: Never commit secret keys or mainnet-specific credentials to the repository.

### 4. Initialize

- [ ] Verify contract address is correct
- [ ] Double-check initialization parameters:
  - Admin address
  - Token address
  - Any other configuration values
- [ ] Initialize contract with verified parameters
- [ ] Record initialization transaction hash
- [ ] Verify initialization succeeded

```bash
# Example initialization (adjust for your contract)
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_SECRET_KEY> \
  --network mainnet \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --token <TOKEN_ADDRESS>
```

### 5. Verify

- [ ] Verify contract source code on block explorer
- [ ] **Protocol Alignment Verification** (see [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md)):
  - [ ] Verify config: `get_config` returns correct token and admin
  - [ ] Verify stream count: `get_stream_count` returns 0
  - [ ] Verify version: `version` returns 1
  - [ ] Test time boundaries: Create stream with past start_time (should fail with `StartTimeInPast`)
  - [ ] Test numeric boundaries: Create stream with zero deposit (should fail with `InvalidParams`)
  - [ ] Test authorization: Non-admin cannot call admin functions (should fail with auth error)
- [ ] Create a small test stream with minimal funds
- [ ] Verify stream creation succeeded: `get_stream_state` returns correct values
- [ ] Verify stream count incremented: `get_stream_count` returns 1
- [ ] Verify recipient index updated: `get_recipient_streams` contains stream_id
- [ ] Test withdrawal functionality with test stream
- [ ] Test pause/resume functionality (if applicable)
- [ ] Verify all events are emitted correctly (query transaction events)
- [ ] Verify token transfers work (check token balances)
- [ ] Monitor contract for first 24-48 hours

### 6. Post-Deployment

- [ ] Update documentation with mainnet contract address
- [ ] Announce deployment to stakeholders
- [ ] Set up monitoring and alerting
- [ ] Establish incident response procedures
- [ ] Document upgrade/migration path (if applicable)
- [ ] Archive deployment artifacts and transaction records

## Emergency Procedures

### If Issues Are Discovered Post-Deployment

1. **Assess Severity**: Determine if the issue poses immediate risk to user funds
2. **Pause Operations**: If admin pause functionality exists, use it to halt new streams
3. **Communicate**: Notify all stakeholders immediately
4. **Document**: Record all details of the issue and actions taken
5. **Plan Migration**: If contract is unusable, plan migration to new deployment

### Admin Key Compromise

1. **Immediate Action**: If possible, transfer admin rights to a new secure key
2. **Pause Contract**: Use pause functionality to prevent further damage
3. **Notify Users**: Communicate the situation transparently
4. **Forensics**: Investigate how the compromise occurred
5. **Recovery Plan**: Deploy new contract if necessary

## Additional Resources

- [Mainnet Deployment Checklist Alignment](./mainnet-deployment-checklist-alignment.md) - Complete verification procedures
- [Protocol Narrative vs Code Alignment](./protocol-narrative-code-alignment.md) - Protocol semantics verification
- [Security Documentation](./security.md)
- [Error Handling](./error.md)
- [Audit Report](./audit.md)
- [Storage Layout](./storage.md)
- [Streaming Mechanics](./streaming.md)
- [Testnet Deployment Guide](./DEPLOYMENT.md)

---

**Remember**: Mainnet deployment is irreversible. Take your time, verify everything, and when in doubt, seek additional review.

## Cross-References

### For Deployers

- **Pre-deployment**: Review [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Pre-Deployment Verification
- **Deployment**: Follow this checklist steps 1-4
- **Post-deployment**: Follow [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Post-Deployment Verification

### For Verifiers

- **Initialization**: [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Initialization Semantics
- **Time boundaries**: [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Time Boundary Verification
- **Numeric ranges**: [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Numeric Range Verification
- **Authorization**: [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md) §Admin Key Verification

### Verification Status

This deployment checklist is verified against protocol semantics in [mainnet-deployment-checklist-alignment.md](./mainnet-deployment-checklist-alignment.md):

- ✅ All deployment steps have observable on-chain verification
- ✅ All authorization boundaries are tested
- ✅ All time boundaries are tested
- ✅ All numeric boundaries are tested
- ✅ All failure modes are documented
- ✅ Zero contradictions with protocol documentation

Last verified: 2026-03-27
