use arrow_schema::DataType;
use arroyo_connectors::{
    nexmark::{NexmarkConnector, NexmarkTable},
    EmptyConfig,
};
use arroyo_operator::connector::Connector;
use arroyo_types::NullableType;

use crate::{parse_and_get_arrow_program, parse_and_get_program, ArroyoSchemaProvider, SqlConfig};

#[ignore]
#[tokio::test]
async fn test_parse() {
    let schema_provider = get_test_schema_provider();

    let sql = "
  WITH bids as (SELECT bid.auction as auction, bid.datetime as datetime
    FROM (select bid from  nexmark) where bid is not null)
    SELECT AuctionBids.auction as auction, AuctionBids.num as count
    FROM (
      SELECT
        B1.auction,
        HOP(INTERVAL '2' SECOND, INTERVAL '10' SECOND) as window,
        count(*) AS num

      FROM bids B1
      GROUP BY
        1,2
    ) AS AuctionBids
    JOIN (
      SELECT
        max(num) AS maxn,
        window
      FROM (
        SELECT
          count(*) AS num,
          HOP(INTERVAL '2' SECOND, INTERVAL '10' SECOND) AS window
        FROM bids B2
        GROUP BY
          B2.auction,2
        ) AS CountBids
      GROUP BY 2
    ) AS MaxBids
    ON
       AuctionBids.num = MaxBids.maxn
       and AuctionBids.window = MaxBids.window;";

    parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_program_compilation() {
    let schema_provider = get_test_schema_provider();

    let sql = "
    SELECT * FROM (
    SELECT ROW_NUMBER()  OVER (
        PARTITION BY window
        ORDER BY count DESC) as row_number, auction FROM (
      SELECT       bid.auction as auction,
    hop(INTERVAL '10' minute, INTERVAL '20' minute ) as window,
    count(*) as count
  FROM (SELECT bid from nexmark where bid is not null)
  GROUP BY 1, 2)) where row_number = 1 ";

    parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_table_alias() {
    let schema_provider = get_test_schema_provider();

    let sql = "SELECT P1.bid FROM nexmark as P1";

    parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

fn get_test_schema_provider() -> ArroyoSchemaProvider {
    let mut schema_provider = ArroyoSchemaProvider::new();

    let nexmark = (NexmarkConnector {})
        .from_config(
            Some(1),
            "nexmark",
            EmptyConfig {},
            NexmarkTable {
                event_rate: 10.0,
                runtime: Some(10.0 * 1_000_000.0),
            },
            None,
        )
        .unwrap();

    schema_provider.add_connector_table(nexmark);

    schema_provider
}

#[tokio::test]
async fn test_window_function() {
    let schema_provider = get_test_schema_provider();

    let sql = "SELECT * FROM (
    SELECT *, ROW_NUMBER() OVER (
        PARTITION BY window
        ORDER BY count DESC) as row_num
    FROM (SELECT count(*) as count,
        hop(interval '2 seconds', interval '10 seconds') as window
            FROM nexmark
            group by window)) WHERE row_num <= 5";

    parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[ignore]
#[tokio::test]
async fn test_no_updating_window_functions() {
    let schema_provider = get_test_schema_provider();
    let sql = "SELECT *, row_number() OVER (partition by bid.auction order by bid.datetime desc) as row_num
     FROM nexmark where bid is not null";
    let err = parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "window function must be partitioned by a window as the first argument"
    );
}

#[tokio::test]
async fn test_no_virtual_fields_updating() {
    let schema_provider = get_test_schema_provider();
    let sql =  "CREATE table debezium_source (
        bids_auction int,
        price int,
        auctions_id int,
        initial_bid int,
        date_string text,
        datetime datetime GENERATED ALWAYS AS (CAST(date_string as timestamp)),
        watermark datetime GENERATED ALWAYS AS (CAST(date_string as timestamp) - interval '1 second')
      ) WITH (
        connector = 'kafka',
        bootstrap_servers = 'localhost:9092',
        type = 'source',
        topic = 'updating',
        format = 'debezium_json'
      );
      SELECT * FROM debezium_source";
    let _ = parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap_err();
}

#[ignore]
#[tokio::test]
async fn test_no_inserting_updates_into_non_updating() {
    let schema_provider = get_test_schema_provider();
    let sql = "CREATE table debezium_source (
        count int,
      ) WITH (
        connector = 'kafka',
        bootstrap_servers = 'localhost:9092',
        type = 'source',
        topic = 'updating',
        format = 'debezium_json'
      );

      CREATE table sink (
        count int
      ) WITH (
        connector = 'kafka',
        bootstrap_servers = 'localhost:9092',
        type = 'sink',
        topic = 'sink',
        format = 'json'
      );

      INSERT into sink
      SELECT * FROM debezium_source";
    let _ = parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap_err();
}

#[ignore]
#[tokio::test]
async fn test_no_aggregates_in_window() {
    let schema_provider = get_test_schema_provider();
    let sql = "WITH bids as (
  SELECT bid.auction as auction, bid.price as price, bid.bidder as bidder, bid.extra as extra, bid.datetime as datetime
  FROM nexmark where bid is not null)

SELECT * FROM (
SELECT bidder, COUNT( distinct auction) as distinct_auctions
FROM bids B1
GROUP BY bidder, HOP(INTERVAL '3 second', INTERVAL '10' minute)) WHERE distinct_auctions > 2";
    let _ = parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_source_rewriter_join() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    let schema_provider = get_test_schema_provider();
    let sql = "CREATE TABLE impulse WITH(
    connector = 'impulse',
    event_rate = '10000'
  ); 
    
  
  SELECT evens.even_counter FROM 
  (SELECT counter as even_counter FROM impulse where counter % 2 = 0) evens
  JOIN impulse on evens.even_counter = impulse.counter;";
    parse_and_get_arrow_program(sql.to_string(), schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_query_parsing() {
    let schema_provider = get_test_schema_provider();
    let sql = "
    CREATE TABLE nexmark WITH (
      connector = 'nexmark',
      event_rate = '10'
    );select bid.datetime
    from nexmark ;";
    parse_and_get_arrow_program(sql.to_string(), schema_provider, SqlConfig::default())
        .await
        .unwrap();
}

#[tokio::test]

async fn test_udf() {
    let mut schema_provider = get_test_schema_provider();

    schema_provider
        .add_rust_udf("fn my_sqr(x: i64) -> i64 { x * x }")
        .unwrap();

    schema_provider
        .add_rust_udf("fn my_sqr_opt(x: i64) -> Option<i64> { Some(x * x) }")
        .unwrap();

    let def = schema_provider.udf_defs.get("my_sqr").unwrap();
    assert_eq!(def.ret, NullableType::not_null(DataType::Int64));

    let def = schema_provider.udf_defs.get("my_sqr_opt").unwrap();
    assert_eq!(def.ret, NullableType::null(DataType::Int64));

    let sql = "SELECT my_sqr(bid.auction), my_sqr_opt(bid.auction) FROM nexmark";
    parse_and_get_program(sql, schema_provider, SqlConfig::default())
        .await
        .unwrap();
}
