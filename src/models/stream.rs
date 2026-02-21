use serde::{Deserialize, Serialize};

/// Parameters needed to construct stream URL requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamParams {
    pub subscription_id: String,
    pub sub_costplan_id_access_list: String,
    pub user_id: String,
    pub start_stamp: String,
    pub end_stamp: String,
}
