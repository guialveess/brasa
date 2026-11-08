//! Importing OpenAPI 3.1, 3.0 and Swagger 2.0 specs (see `docs/openapi-import.md`).
//!
//! Pure like the rest of `brasa-format`: [`parse_spec`] reads spec text (JSON or YAML) into a
//! normalized [`Spec`], loading any external `$ref` through the caller's [`RefLoader`], and
//! [`build_plan`] turns one or more specs and the [`ImportOptions`] into an [`OpenApiPlan`].
//! `brasa-workspace` writes the plan into a workspace.

mod catalog;
mod document;
mod error;
mod example;
mod generate;
mod manifest;
mod spec;

pub use catalog::{
    CatalogApi, CatalogDefinition, looks_like_catalog, looks_like_spec, parse_catalog,
};
pub use document::{NoExternalRefs, RefLoader};
pub use error::OpenApiError;
pub use generate::{
    Grouping, ImportOptions, OpenApiPlan, PlanEnvironment, PlanFolder, PlanRequest, SpecVersion,
    build_plan,
};
pub use manifest::{
    ChangeKind, MANIFEST_FILE, Manifest, ManifestGrouping, ManifestOptions, ManifestRequest,
    ManifestSource, PlanChange, diff_plan, fingerprint, update_request,
};
pub use spec::{
    Operation, Parameter, ParameterLocation, RequestBody, SchemeKind, SecurityScheme, Server, Spec,
    Success, parse_spec,
};
