# Fluxora Contracts Events

This document lists all the events emitted by the Fluxora payment streaming contract. These events can be used by indexers and off-chain systems to track the state of streams and administrative changes.

## Stream Lifecycle Events

All stream lifecycle events use the project-wide sequential `stream_id` as part of the topic.

### created
Emitted when a new payment stream is created.
- **Topic**: `(symbol_short!("created"), stream_id)`
- **Data**: `deposit_amount` (i128)

### paused
Emitted when an active stream is paused.
- **Topic**: `(symbol_short!("paused"), stream_id)`
- **Data**: `StreamEvent::Paused(stream_id)` (Enum)

### resumed
Emitted when a paused stream is resumed.
- **Topic**: `(symbol_short!("resumed"), stream_id)`
- **Data**: `StreamEvent::Resumed(stream_id)` (Enum)

### cancelled
Emitted when a stream is cancelled (early termination).
- **Topic**: `(symbol_short!("cancelled"), stream_id)`
- **Data**: `StreamEvent::Cancelled(stream_id)` (Enum)

### withdrew
Emitted when the recipient withdraws tokens from a stream.
- **Topic**: `(symbol_short!("withdrew"), stream_id)`
- **Data**: `amount` (i128)

## Administrative Events

### AdminUpdated
Emitted when the contract admin is changed.
- **Topic**: `(Symbol::new(&env, "AdminUpdated"),)`
- **Data**: `(old_admin, new_admin)` (tuple of Addresses)
