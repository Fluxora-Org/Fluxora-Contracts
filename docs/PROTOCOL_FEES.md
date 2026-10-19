# Protocol fee policy

Fluxora Contracts do not charge a protocol fee.

The factory and stream contracts do not custody or deduct a platform fee from
deposits, withdrawals, refunds, or stream settlements. Users still pay the
normal Stellar network resource fee for transactions; that network fee is not
collected or controlled by Fluxora.

This is an intentional design decision, not an unimplemented runtime switch:

- there is no fee recipient or fee rate stored in contract state;
- settlement amounts are calculated from the stream terms and do not include a
  hidden deduction; and
- changing this policy would require a separately versioned contract design,
  explicit migration guidance, and updated client accounting.

Integrators should therefore display the stream amounts as specified by the
sender and recipient, while presenting Stellar transaction costs separately.
