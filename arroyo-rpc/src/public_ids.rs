use nanoid::nanoid;

const ID_LENGTH: usize = 10;

const ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b',
    'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u',
    'v', 'w', 'x', 'y', 'z',
];

pub enum IdTypes {
    ApiKey,
    Connection,
    Schema,
    Pipeline,
    PipelineDefinition,
    JobConfig,
    Checkpoint,
    JobStatus,
    ClusterInfo,
    JobLogMessage,
    ConnectionTable,
    ConnectionTablePipeline,
}

pub fn generate_id(id_type: IdTypes) -> String {
    let prefix = match id_type {
        IdTypes::ApiKey => "ak",
        IdTypes::Connection => "conn",
        IdTypes::Schema => "sch",
        IdTypes::Pipeline => "pl",
        IdTypes::PipelineDefinition => "pld",
        IdTypes::JobConfig => "jc",
        IdTypes::Checkpoint => "cp",
        IdTypes::JobStatus => "js",
        IdTypes::ClusterInfo => "ci",
        IdTypes::JobLogMessage => "jlm",
        IdTypes::ConnectionTable => "ct",
        IdTypes::ConnectionTablePipeline => "ctp",
    };
    let id = nanoid!(ID_LENGTH, &ALPHABET);
    format!("{}_{}", prefix, id)
}
