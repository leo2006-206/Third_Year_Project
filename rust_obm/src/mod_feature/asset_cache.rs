/// The offload site's cache of show assets (specs, images, source video segments it fetches
/// from the main server). Just a PolicyCache: key = resource path, value = response body.
pub type AssetCache = crate::mod_feature::PolicyCache;
