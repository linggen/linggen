//! Background upkeep of a hosted store (`maintenance`). The daemon process
//! that used to live here (serve, start, stop, its pidfile) is gone: memory
//! is hosted by the Linggen engine (`crate::hosted`).

pub mod maintenance;
