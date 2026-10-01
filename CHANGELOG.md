# Changelog

All notable changes to the Fluxora stream contract will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Stream Reference Field** ([#1816](https://github.com/Fluxora-Org/Fluxora-Contracts/issues/1816))
  - Added optional `reference` field to `Stream` struct for on-chain stream identification
  - New `reference` parameter in `create_stream` function (Option<String>)
  - Maximum reference length of 64 characters (`MAX_REFERENCE_LENGTH` constant)
  - Reference field included in `StreamCreated` events and returned by `get_stream`
  - New `InvalidReferenceLength` (40) error for references exceeding maximum length
  - Comprehensive validation tests for empty, valid, maximum-length, and over-length references
  - Storage cost analysis documentation showing 1-73 byte overhead depending on reference length

### Technical Details
- Reference field is stored as `Option<String>` in persistent storage with each stream
- Validation occurs at creation time before token transfer
- Field is immutable after stream creation
- Storage cost: +1 byte (None) to +73 bytes (64-character reference)
- Compatible with existing streams (defaults to None for backward compatibility)
