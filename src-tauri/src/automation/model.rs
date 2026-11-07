use super::*;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Data {
    pub enabled: bool,
    pub clients: Vec<Client>,
    pub roots: Vec<OutputRoot>,
    pub plans: Vec<Plan>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    pub capabilities: Vec<String>,
    pub collections: Vec<String>,
    pub output_root_ids: Vec<String>,
    pub max_bytes: u64,
    pub max_seconds: u32,
    #[serde(default)]
    pub include_documents: bool,
}
impl Default for Grant {
    fn default() -> Self {
        Self {
            capabilities: vec![],
            collections: vec![],
            output_root_ids: vec![],
            max_bytes: 128 * MIN_BYTES,
            max_seconds: 120,
            include_documents: false,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientState {
    Pending,
    Authorized,
    Revoked,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub state: ClientState,
    pub created_at: u64,
    #[serde(flatten)]
    pub grant: Grant,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputRoot {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanState {
    Pending,
    Approved,
    Denied,
    Consumed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budget {
    pub max_bytes: u64,
    pub max_seconds: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Operation {
    Download {
        url: String,
        format: crate::model::Format,
    },
    ExtractAudio {
        library_id: String,
        format: String,
    },
    Frame {
        library_id: String,
        at: f64,
        width: u32,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInput {
    pub operation: Operation,
    pub output_root_id: String,
    pub collection: String,
    pub budget: Budget,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub state: PlanState,
    pub operation: Operation,
    pub output_root_id: String,
    pub output_label: String,
    pub output_path: PathBuf,
    pub collection: String,
    pub budget: Budget,
    pub digest: String,
    pub job_id: Option<String>,
    pub source: queue::Source,
    pub input_hash: Option<String>,
    pub title: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobAuthorization {
    pub client_id: String,
    pub plan_id: String,
    pub digest: String,
}
