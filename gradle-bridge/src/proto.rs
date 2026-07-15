//! Generated gRPC bindings for the shipped `gradle-server` contract.
//!
//! The contents of [`gradle`] are produced by `prost`/`tonic` from
//! `proto/gradle.proto` (a verbatim copy of the proto embedded in
//! `gradle-server.jar`) and committed under `src/gen/`, so the build needs no
//! `protoc`. See `proto/gradle.proto` for regeneration instructions.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod gradle {
    include!("gen/gradle.rs");
}
