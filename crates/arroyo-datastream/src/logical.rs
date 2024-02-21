use datafusion_proto::protobuf::ArrowType;

use arroyo_rpc::df::ArroyoSchema;
use arroyo_rpc::grpc::api;
use arroyo_rpc::grpc::api::{
    ArrowDylibUdfConfig, ArrowProgram, ArrowProgramConfig, EdgeType, JobEdge, JobGraph, JobNode,
};
use petgraph::graph::DiGraph;
use petgraph::prelude::EdgeRef;
use petgraph::Direction;
use prost::Message;
use rand::distributions::Alphanumeric;
use rand::prelude::SmallRng;
use rand::{Rng, SeedableRng};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::hash::Hasher;
use strum::{Display, EnumString};

#[derive(Clone, Copy, Debug, Eq, PartialEq, EnumString, Display)]
pub enum OperatorName {
    ExpressionWatermark,
    ArrowValue,
    ArrowKey,
    ArrowAggregate,
    Join,
    InstantJoin,
    TumblingWindowAggregate,
    SlidingWindowAggregate,
    SessionWindowAggregate,
    ConnectorSource,
    ConnectorSink,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum LogicalEdgeType {
    Forward,
    Shuffle,
    LeftJoin,
    RightJoin,
}

impl Display for LogicalEdgeType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LogicalEdgeType::Forward => write!(f, "→"),
            LogicalEdgeType::Shuffle => write!(f, "⤨"),
            LogicalEdgeType::LeftJoin => write!(f, "-[left]⤨"),
            LogicalEdgeType::RightJoin => write!(f, "-[right]⤨"),
        }
    }
}

impl From<arroyo_rpc::grpc::api::EdgeType> for LogicalEdgeType {
    fn from(value: EdgeType) -> Self {
        match value {
            EdgeType::Unused => panic!("invalid edge type"),
            EdgeType::Forward => LogicalEdgeType::Forward,
            EdgeType::Shuffle => LogicalEdgeType::Shuffle,
            EdgeType::LeftJoin => LogicalEdgeType::LeftJoin,
            EdgeType::RightJoin => LogicalEdgeType::RightJoin,
        }
    }
}

impl From<LogicalEdgeType> for api::EdgeType {
    fn from(value: LogicalEdgeType) -> Self {
        match value {
            LogicalEdgeType::Forward => EdgeType::Forward,
            LogicalEdgeType::Shuffle => EdgeType::Shuffle,
            LogicalEdgeType::LeftJoin => EdgeType::LeftJoin,
            LogicalEdgeType::RightJoin => EdgeType::RightJoin,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalEdge {
    pub edge_type: LogicalEdgeType,
    pub schema: ArroyoSchema,
    pub projection: Option<Vec<usize>>,
}

impl LogicalEdge {
    pub fn new(
        edge_type: LogicalEdgeType,
        schema: ArroyoSchema,
        projection: Option<Vec<usize>>,
    ) -> Self {
        LogicalEdge {
            edge_type,
            schema,
            projection,
        }
    }

    pub fn project_all(edge_type: LogicalEdgeType, schema: ArroyoSchema) -> Self {
        LogicalEdge {
            edge_type,
            schema,
            projection: None,
        }
    }
}

#[derive(Clone)]
pub struct LogicalNode {
    pub operator_id: String,
    pub description: String,
    pub operator_name: OperatorName,
    pub operator_config: Vec<u8>,
    pub parallelism: usize,
}

impl Display for LogicalNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}

impl Debug for LogicalNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.operator_id)
    }
}

pub type LogicalGraph = DiGraph<LogicalNode, LogicalEdge>;

#[derive(Clone, Debug)]
pub struct DylibUdfConfig {
    pub dylib_path: String,
    pub arg_types: Vec<ArrowType>,
    pub return_type: ArrowType,
}

#[derive(Clone, Debug)]
pub struct ProgramConfig {
    pub udf_dylibs: HashMap<String, DylibUdfConfig>,
}

#[derive(Clone, Debug)]
pub struct LogicalProgram {
    pub graph: LogicalGraph,
    pub program_config: ProgramConfig,
}

impl LogicalProgram {
    pub fn update_parallelism(&mut self, overrides: &HashMap<String, usize>) {
        for node in self.graph.node_weights_mut() {
            if let Some(p) = overrides.get(&node.operator_id) {
                node.parallelism = *p;
            }
        }
    }

    pub fn task_count(&self) -> usize {
        // TODO: this can be cached
        self.graph.node_weights().map(|nw| nw.parallelism).sum()
    }

    pub fn sources(&self) -> HashSet<&str> {
        // TODO: this can be memoized
        self.graph
            .externals(Direction::Incoming)
            .map(|t| self.graph.node_weight(t).unwrap().operator_id.as_str())
            .collect()
    }

    pub fn get_hash(&self) -> String {
        let mut hasher = DefaultHasher::new();
        let bs = api::ArrowProgram::from(self.clone()).encode_to_vec();
        for b in bs {
            hasher.write_u8(b);
        }

        let rng = SmallRng::seed_from_u64(hasher.finish());

        rng.sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }

    pub fn tasks_per_operator(&self) -> HashMap<String, usize> {
        let mut tasks_per_operator = HashMap::new();
        for node in self.graph.node_weights() {
            tasks_per_operator.insert(node.operator_id.clone(), node.parallelism);
        }
        tasks_per_operator
    }

    pub fn as_job_graph(&self) -> JobGraph {
        let nodes = self
            .graph
            .node_weights()
            .map(|node| JobNode {
                node_id: node.operator_id.to_string(),
                operator: node.description.clone(),
                parallelism: node.parallelism as u32,
            })
            .collect();

        let edges = self
            .graph
            .edge_references()
            .map(|edge| {
                let src = self.graph.node_weight(edge.source()).unwrap();
                let target = self.graph.node_weight(edge.target()).unwrap();
                JobEdge {
                    src_id: src.operator_id.to_string(),
                    dest_id: target.operator_id.to_string(),
                    key_type: "()".to_string(),
                    value_type: "()".to_string(),
                    edge_type: format!("{:?}", edge.weight().edge_type),
                }
            })
            .collect();

        JobGraph { nodes, edges }
    }
}

impl TryFrom<ArrowProgram> for LogicalProgram {
    type Error = anyhow::Error;

    fn try_from(value: ArrowProgram) -> anyhow::Result<Self> {
        let mut graph = DiGraph::new();

        let mut id_map = HashMap::new();

        for node in value.nodes {
            id_map.insert(
                node.node_index,
                graph.add_node(LogicalNode {
                    operator_id: node.node_id,
                    description: node.description,
                    operator_name: OperatorName::try_from(node.operator_name.as_str())?,
                    operator_config: node.operator_config,
                    parallelism: node.parallelism as usize,
                }),
            );
        }

        for edge in value.edges {
            let source = *id_map.get(&edge.source).unwrap();
            let target = *id_map.get(&edge.target).unwrap();
            let schema = edge.schema.as_ref().unwrap();

            graph.add_edge(
                source,
                target,
                LogicalEdge {
                    edge_type: edge.edge_type().into(),
                    schema: ArroyoSchema {
                        schema: serde_json::from_str(&schema.arrow_schema).unwrap(),
                        timestamp_index: schema.timestamp_index as usize,
                        key_indices: schema.key_indices.iter().map(|t| *t as usize).collect(),
                    },
                    projection: if edge.projection.is_empty() {
                        None
                    } else {
                        Some(edge.projection.iter().map(|p| *p as usize).collect())
                    },
                },
            );
        }

        let program_config = value
            .program_config
            .unwrap_or_else(|| ArrowProgramConfig {
                udf_dylibs: HashMap::new(),
            })
            .into();

        Ok(LogicalProgram {
            graph,
            program_config,
        })
    }
}

impl From<DylibUdfConfig> for ArrowDylibUdfConfig {
    fn from(from: DylibUdfConfig) -> Self {
        ArrowDylibUdfConfig {
            dylib_path: from.dylib_path,
            arg_types: from.arg_types.iter().map(|t| t.encode_to_vec()).collect(),
            return_type: from.return_type.encode_to_vec(),
        }
    }
}

impl From<ArrowDylibUdfConfig> for DylibUdfConfig {
    fn from(from: ArrowDylibUdfConfig) -> Self {
        DylibUdfConfig {
            dylib_path: from.dylib_path,
            arg_types: from
                .arg_types
                .iter()
                .map(|t| ArrowType::decode(&mut t.as_slice()).expect("invalid arrow type"))
                .collect(),
            return_type: ArrowType::decode(&mut from.return_type.as_slice())
                .expect("invalid arrow type"),
        }
    }
}

impl From<ProgramConfig> for ArrowProgramConfig {
    fn from(from: ProgramConfig) -> Self {
        ArrowProgramConfig {
            udf_dylibs: from
                .udf_dylibs
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}

impl From<ArrowProgramConfig> for ProgramConfig {
    fn from(from: ArrowProgramConfig) -> Self {
        ProgramConfig {
            udf_dylibs: from
                .udf_dylibs
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}

impl From<LogicalProgram> for ArrowProgram {
    fn from(value: LogicalProgram) -> Self {
        let graph = value.graph;
        let nodes = graph
            .node_indices()
            .map(|idx| {
                let node = graph.node_weight(idx).unwrap();
                api::ArrowNode {
                    node_index: idx.index() as i32,
                    node_id: node.operator_id.clone(),
                    parallelism: node.parallelism as u32,
                    description: node.description.clone(),
                    operator_name: node.operator_name.to_string(),
                    operator_config: node.operator_config.clone(),
                }
            })
            .collect();

        let edges = graph
            .edge_indices()
            .map(|idx| {
                let edge = graph.edge_weight(idx).unwrap();
                let (source, target) = graph.edge_endpoints(idx).unwrap();

                let edge_type: api::EdgeType = edge.edge_type.into();
                api::ArrowEdge {
                    source: source.index() as i32,
                    target: target.index() as i32,
                    schema: Some(api::ArroyoSchema {
                        arrow_schema: serde_json::to_string(&edge.schema.schema).unwrap(),
                        timestamp_index: edge.schema.timestamp_index as u32,
                        key_indices: edge.schema.key_indices.iter().map(|k| *k as u32).collect(),
                    }),
                    edge_type: edge_type as i32,
                    projection: edge
                        .projection
                        .as_ref()
                        .map(|p| p.iter().map(|v| *v as u32).collect())
                        .unwrap_or(vec![]),
                }
            })
            .collect();

        api::ArrowProgram {
            nodes,
            edges,
            program_config: Some(value.program_config.into()),
        }
    }
}
