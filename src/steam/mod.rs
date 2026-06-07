//! Distribution re-exports — achievements and platform client.

pub use mahjuro_distribution::Achievement;
pub use mahjuro_distribution::{
    DistributionBackend, DistributionClient, DistributionConfig,
};
pub use mahjuro_distribution::{PlatformPaths, PlatformShell};

#[cfg(feature = "dist-steam")]
pub use mahjuro_distribution::steam::steamworks_dll_ready;
