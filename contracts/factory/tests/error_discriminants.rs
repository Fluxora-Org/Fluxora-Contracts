//! ABI compatibility coverage for the public factory error codes.

use fluxora_factory::FactoryError;

#[test]
fn factory_error_discriminants_are_pinned_and_unique() {
    let discriminants = [
        FactoryError::AlreadyInitialized as u32,
        FactoryError::NotInitialized as u32,
        FactoryError::Unauthorized as u32,
        FactoryError::InvalidCap as u32,
        FactoryError::InvalidMinDuration as u32,
        FactoryError::InvalidRateBounds as u32,
        FactoryError::FactoryPaused as u32,
    ];

    assert_eq!(
        discriminants,
        [1, 2, 3, 4, 5, 6, 7],
        "FactoryError codes are part of the contract ABI"
    );

    for (index, code) in discriminants.iter().enumerate() {
        assert!(
            discriminants[..index]
                .iter()
                .all(|previous| previous != code),
            "FactoryError discriminant {code} is duplicated"
        );
    }
}
