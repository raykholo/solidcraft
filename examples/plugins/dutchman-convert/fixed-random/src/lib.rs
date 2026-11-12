//! Fixed "random" bytes, standing in for the `getrandom` crate in this plugin's builds.
//!
//! An importer has no real randomness: SolidCraft gives the desktop component fixed bytes, and
//! the browser's web ABI gives a module nothing at all. The real crate would reach for the
//! browser's crypto through JavaScript glue, which the browser host does not give, so the
//! plugin's Cargo.toml patches this in instead: the same functions, filling with a fixed
//! pattern, so one file converts the same way every time. The plugin's dependencies only use
//! these bytes for names of temporary entries, never for security.

use core::mem::MaybeUninit;

/// The error type of the functions below (they never fail).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(u32);

/// An operating system's error code.
pub type RawOsError = i32;

impl Error {
    /// A custom error code, as the real crate offers.
    pub const fn new_custom(n: u16) -> Error {
        Error(n as u32)
    }
    /// Never an operating system's error here.
    pub fn raw_os_error(self) -> Option<RawOsError> {
        None
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "fixed random bytes: error {}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        std::io::Error::other(e)
    }
}

/// Fill `dest` with the fixed pattern.
pub fn fill(dest: &mut [u8]) -> Result<(), Error> {
    for (i, b) in dest.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31).wrapping_add(7);
    }
    Ok(())
}

/// Fill `dest` with the fixed pattern; the filled bytes.
pub fn fill_uninit(dest: &mut [MaybeUninit<u8>]) -> Result<&mut [u8], Error> {
    for (i, b) in dest.iter_mut().enumerate() {
        b.write((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    // SAFETY: every byte was just written.
    Ok(unsafe { &mut *(dest as *mut [MaybeUninit<u8>] as *mut [u8]) })
}

/// A fixed `u32`.
pub fn u32() -> Result<u32, Error> {
    let mut b = [0u8; 4];
    fill(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// A fixed `u64`.
pub fn u64() -> Result<u64, Error> {
    let mut b = [0u8; 8];
    fill(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
