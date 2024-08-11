#[cfg(feature = "log")]
pub use log::{warn, info, error, debug, trace};
#[cfg(feature = "defmt")]
pub use defmt::{warn, info, debug, trace, error};
