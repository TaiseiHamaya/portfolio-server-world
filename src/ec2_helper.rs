use std::{net::Ipv4Addr, str::FromStr};

use aws_config::imds::Client as ImdsClient;

pub async fn get_local_ip() -> Ipv4Addr {
    let imds_client = ImdsClient::builder().build();

    let Ok(local_ipv4) = imds_client.get("/latest/meta-data/local-ipv4").await else {
        return Ipv4Addr::LOCALHOST;
    };

    let local_ipv4 = Ipv4Addr::from_str(local_ipv4.as_ref()).unwrap_or(Ipv4Addr::LOCALHOST);
    local_ipv4
}
