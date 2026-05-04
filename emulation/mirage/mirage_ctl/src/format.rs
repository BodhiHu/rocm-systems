#[derive(serde::Serialize, serde::Deserialize, Default)]
pub enum OutputFormat {
    #[default]
    Default,
    Json,
}
