use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the Callora Topics contract.
///
/// | Code | Variant              | Meaning                                          |
/// |------|----------------------|--------------------------------------------------|
/// | 1    | NotInitialized       | Contract has not been initialized yet            |
/// | 2    | AlreadyInitialized   | `init` was called more than once                 |
/// | 3    | Unauthorized         | Caller is not the admin                          |
/// | 4    | TopicAlreadyExists   | Topic with that name is already registered       |
/// | 5    | TopicNotFound        | No topic with that name exists                   |
/// | 6    | Overflow             | Arithmetic overflow in topic counter             |
/// | 7    | InvalidDescription   | Description is empty, too long (> 256 bytes), or contains non-visible-ASCII bytes |
/// | 8    | TopicAlreadyInactive | Topic is already deactivated                     |
/// | 9    | SameOwner            | New topic owner equals the current owner         |
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TopicsError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// Contract has already been initialized (code 2).
    AlreadyInitialized = 2,
    /// Caller is not the admin (code 3).
    Unauthorized = 3,
    /// Topic with that name is already registered (code 4).
    TopicAlreadyExists = 4,
    /// No topic with that name exists (code 5).
    TopicNotFound = 5,
    /// Arithmetic overflow (code 6).
    Overflow = 6,
    /// Description is empty, exceeds 256 bytes, contains control characters,
    /// or has leading / trailing whitespace (code 7).
    ///
    /// Validated by [`callora_validators::normalize_visible_ascii`].
    InvalidDescription = 7,
    /// Topic is already deactivated; `deactivate` was called twice (code 8).
    TopicAlreadyInactive = 8,
    /// `set_topic_owner` was called with the current owner (code 9).
    SameOwner = 9,
}
