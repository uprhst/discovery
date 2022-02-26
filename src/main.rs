macro_rules! repeat_until_ok {
    ($func: expr) => {
        {            
            let value = loop {
                let now = std::time::Instant::now();

                let res = $func;

                if let Err(err) = res {
                    eprintln!("{}", err);
                } else {
                    break res;
                }

                let duration = std::time::Instant::now().duration_since(now).as_millis() as u64;

                if duration < 5000 {
                    eprintln!("Sleeping for {} ms", 5000 - duration);
                    tokio::time::sleep(std::time::Duration::from_millis(5000 - duration)).await;
                }
            };

            value
        }
    };
}

pub mod network;

use minreq;
use network::{ping, Network};

use serde::{Serialize, Deserialize};
use serde_with::{serde_as, DefaultOnError};

use futures::StreamExt;
use futures::stream::TryStreamExt;

use version_compare::{Cmp, Version as AsNumeric};

use std::ops::Add;

use tonic::{transport::Server, Request, Response, Status};

pub mod dsc_funcs { tonic::include_proto!("discovery"); }
pub mod node_funcs { tonic::include_proto!("node_funcs"); }

use dsc_funcs::{
    discovery_client::DiscoveryClient,
    discovery_server::{Discovery, DiscoveryServer}
};
use dsc_funcs::{Net, Empty};

use node_funcs::node_maistrou_client::NodeMaistrouClient;
use node_funcs::{PageRequest, ListReply};

#[derive(Debug, Clone)]
pub struct DiscoveryRPC {
    router: bool,
    network: Option<Network>,
    location: String,
    registered: bool
}

unsafe impl Send for DiscoveryRPC {}
unsafe impl Sync for DiscoveryRPC {}

impl DiscoveryRPC {
    pub fn new(router: bool, metadata: Metadata) -> Result<Self, Box<dyn std::error::Error>> {
        let network = if !router {
            let interfaces: Vec<&Interfaces> = metadata.interfaces.iter().filter(|i| i.r#type == "private").collect();

            if interfaces.len() > 1 {
                return Err("Cannot have more than one interface on a normal server!".into());
            }

            let address = interfaces[0].ipv4.address.parse::<std::net::Ipv4Addr>()?;
            let netmask = interfaces[0].ipv4.netmask.parse::<std::net::Ipv4Addr>()?;

            Some(Network(ipnetwork::Ipv4Network::with_netmask(address, netmask)?))
        } else { None };

        Ok(
            Self {
                router: router,
                network,
                location: metadata.region["regioncode"].clone(),
                registered: false
            }
        )
    }

    pub async fn router_announcement(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // A router cannot announce it is up and running,
        // there is a separate functionality to inform
        // routers in other locations know about me
        if self.router {
            return Ok(());
        }

        let network = self.network.as_ref().unwrap();

        let router = network.gateway().map_err(|err| format!("{}", err))?;

        let mut client = repeat_until_ok!(DiscoveryClient::connect(format!("http://{}:50051", router)).await
        .map_err(|err| format!("Failed connecting to Discovery Router: {}: {}", router, err)))?;

        let response = repeat_until_ok!(
            client.announce(
                tonic::Request::new(
                    Net {
                        address: network.addr().to_string(),
                        prefix: network.prefix() as i32,
                        location: self.location.clone(),
                    }
                )
            ).await
        )?.into_inner();

        println!("The response we received was: {:?}", response);

        Ok(())
    }

    pub async fn ping_discovery(&self, address: std::net::Ipv4Addr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

        let mut client = repeat_until_ok!(DiscoveryClient::connect(format!("http://{}:50051", address)).await
        .map_err(|err| format!("Failed connecting to Discovered Server: {}: {}", address, err)))?;

        let response = repeat_until_ok!(
            client.ping(
                tonic::Request::new(
                    Empty {}
                )
            ).await
        )?.into_inner();

        println!("The response we recieved was: {:?}", response);

        Ok(())
    }

    pub async fn pinger(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

        if ! self.router {
            return Ok(());
        }

        let mut map: std::collections::HashMap<String, (std::net::Ipv4Addr, std::time::Instant)> = std::collections::HashMap::new();
    
        for i in 0..=20000 {
            map.insert(format!("server{}", i), ("10.0.0.4".parse()?, std::time::Instant::now()));
        }
        
        let mut _map = map;

        loop {
            let mut index = 0;

            let now = std::time::Instant::now();

            let map = _map.iter_mut().filter(|(_,(_,ts))| std::time::Instant::now().duration_since(*ts) > std::time::Duration::from_secs(5)).map(|(name,(address, ts))| { *ts = std::time::Instant::now() + std::time::Duration::from_secs(10); (name.clone(), *address)}).collect::<Vec<(String, std::net::Ipv4Addr)>>();

            futures::stream::iter(map.into_iter().map(|(name, address)| {
                let name = name.to_string();
                let address = address.to_string();
                tokio::spawn(async move {
                    let will_sleep = rand::random::<u8>() as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(will_sleep)).await;

                    (name, DiscoveryClient::connect(format!("http://{}:50051", address)).await
                    .map_err(|err| format!("Failed connecting to Discovered Server: {}: {}", address, err)))
                })
            }))
            .buffer_unordered(100)
            .map(|res| {
                index += 1;

                if let Err(err) = res {
                    eprintln!("{}", err);
                    return ();
                }

                let (name, res) = res.unwrap();

                if let Err(err) = res {
                    eprintln!("{} has failed, we will need to retry it later {}", name, err);
                    return ();
                }

                println!("[{:?} | {}] connection successful", 
                std::thread::current().id(),
                std::thread::current().name().get_or_insert("DfT"));
            })
            .collect::<Vec<_>>()
            .await;

            let will_sleep = 4000 + (rand::random::<u8>() as u64) * 2;

            println!("[{:?} | {}] ({}) Elapsed time: {:.2?} .. will sleep: {}", 
            std::thread::current().id(),
            std::thread::current().name().get_or_insert("DfT"),
            index, now.elapsed(), will_sleep);

            tokio::time::sleep(std::time::Duration::from_millis(will_sleep)).await;
        }

        Ok(())
    }
}

#[tonic::async_trait]
impl Discovery for DiscoveryRPC {
    async fn announce(&self, request: Request<Net>) -> Result<Response<Empty>, Status> {

        println!("Announce request: {:?}", request);

        let request = request.into_inner();

        println!("Announce got: {:?}", request);

        self.ping_discovery(request.address.parse::<std::net::Ipv4Addr>().map_err(|err| Status::internal(err.to_string()))?).await
        .map_err(|err| Status::internal(err.to_string()))?;

        return Ok(Response::new(Empty {}));
        
        unimplemented!()
    }

    async fn ping(&self, request: Request<Empty>) -> Result<Response<Empty>, Status> {

        println!("Ping request: {:?}", request);

        let request = request.into_inner();

        println!("Ping got: {:?}", request);

        return Ok(Response::new(Empty {}));

        unimplemented!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename(deserialize = "instance-v2-id"))]
    id: String,
    hostname: String,
    interfaces: Vec<Interfaces>,
    region: std::collections::HashMap<String, String>
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct Interfaces {
    #[serde(rename(deserialize = "network-v2-id"))]
    id: Option<String>,
    mac: String,
    #[serde(rename(deserialize = "network-type"))]
    r#type: String,
    ipv4: Address,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    ipv6: Option<Address>
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct Address {
    address: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    netmask: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    network: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    prefix: String,
    additional: Option<Vec<Address>>
}


impl Metadata {
    pub async fn setup_tunnel(&self, bridge_iface: &str, routers: Vec<node_funcs::Node>) -> Result<(), Box<dyn std::error::Error>> {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        
        tokio::spawn(connection);

        let bridge = {
            let mut private_link = handle.link().get().match_name(bridge_iface.into()).execute();

            if let Some(link) = private_link.try_next().await? {
                link.header.index
            } else {
                return Err("Could not find `private` interface link index".into());
            }
        };

        let local_addr6 = if let Some(index) = self.interfaces.iter().position(|i| i.ipv6.is_some()) {
            &self.interfaces[index].ipv6.as_ref().unwrap().network
        } else {
            return Err("Could not find `public` IPv6 network to set for tunnel".into());
        };

        let sdn = {
            let mut exists = false;

            let mut sdn_link = handle.link().get().match_name("sdn".into()).execute();

            if let Ok(next) = sdn_link.try_next().await {
                if let Some(_) = next {
                    exists = true;
                }
            }

            exists
            // if let Some(link) = sdn_link.try_next().await? {
            //     true
            // } else {
            //     false
            // }
        };

        println!("SDN IS: {}", sdn);

        if !sdn {
            // 877473   = uprise
            // 8774     = upri
            handle.link().add().vxlan("sdn".into(), 877473u32)
            .port(8774)
            .local6(local_addr6.parse()?)
            // .learning(1)
            .execute().await
            .map_err(|err| format!("Failed creating vxlan interface: type vxlan id 877473 dstport 8774 local {}: {:?}", local_addr6, err))?;
        }

        let sdn = {
            let mut sdn_link = handle.link().get().match_name("sdn".into()).execute();

            if let Some(link) = sdn_link.try_next().await? {
                link.header.index
            } else {
                return Err("Could not find `sdn` vxlan interface".into());
            }
        };

        // Then we set the master and UP! the interface
        handle.link().set(sdn).master(bridge).up().execute().await
        .map_err(|err| format!("Failed setting vxlan interface master to: {} and UP state: {:?}", bridge_iface, err))?;

        // finally, we need to iterate through the available routers
        // and create entries in forwarding database
        for router in &routers {
            handle.neighbours().add_bridge(sdn, &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
            .destination(router.ipv6network.parse()?)
            .state(rtnetlink::packet::rtnl::constants::NUD_NOARP | rtnetlink::packet::rtnl::constants::NUD_PERMANENT)
            .flags(rtnetlink::packet::rtnl::constants::NTF_SELF)
            .append()
            .execute().await
            .map_err(|err| format!("Failed creating entry in forwarding database: 00:00:00:00:00:00 dev sdn dst {} : {:?}", router.ipv6network, err))?;
        }

        Ok(())
    }

    pub async fn add_route(&self, bridge_iface: &str, addr: std::net::Ipv4Addr, prefix: u8) -> Result<(), Box<dyn std::error::Error>> {
        // RouteMessage {
        //     header: RouteHeader {
        //         address_family: 2,
        //         destination_prefix_length: 8,
        //         source_prefix_length: 0,
        //         tos: 0,
        //         table: 254,
        //         protocol: 3,
        //         scope: 253,
        //         kind: 1,
        //         flags: (empty) },
        //     nlas: [
        //             Table(254),
        //             Destination([10, 0, 0, 0]),
        //             Oif(6)
        //         ]
        // }

        let (connection, handle, _) = rtnetlink::new_connection()?;
        
        tokio::spawn(connection);
        
        let bridge = {
            let mut private_link = handle.link().get().match_name(bridge_iface.into()).execute();

            if let Some(link) = private_link.try_next().await? {
                link.header.index
            } else {
                return Err("Could not find `private` interface link index".into());
            }
        };

        // output interface = bridge
        // let table = 254 (main table);
        let proto = 4;
        let scope = 253;

        // Add IPv4 route
        handle.route().add().v4()
        .output_interface(bridge)
        .scope(scope)
        .protocol(proto)
        .destination_prefix(addr, prefix)
        .execute().await
        .map_err(|err| format!("Failed adding route: {}/{} dev {}: {:?}", addr, prefix, bridge_iface, err))?;

        Ok(())
    }

    pub async fn enable_priv_ifaces(&self, bridge_iface: &str) -> Result<(), Box<dyn std::error::Error>> {

        let (connection, handle, _) = rtnetlink::new_connection()?;

        tokio::spawn(connection);

        let bridge = {
            let mut private_link = handle.link().get().match_name(bridge_iface.into()).execute();

            if let Some(link) = private_link.try_next().await? {
                link.header.index
            } else {
                return Err("Could not find `private` interface link index".into());
            }
        };

        let mut links = handle.link().get().execute();

        'outer: while let Some(link) = links.try_next().await? {
            for nla in link.nlas.into_iter() {
                if let rtnetlink::packet::rtnl::link::nlas::Nla::IfName(name) = nla {
                    if !name.contains("eth") {
                        continue 'outer;
                    }

                    let order = name.split("eth").last().ok_or("Could not find interface index")?.parse::<usize>()?;

                    if !(order > 0) {
                        continue 'outer;
                    }

                    handle.link().set(link.header.index).master(bridge).up().execute().await
                    .map_err(|err| format!("Failed setting {} interface master to: {} and UP state: {:?}", name, bridge_iface, err))?;
                }
            }
        }

        Ok(())
    }

    pub async fn setup_priv_addresses(&self, link: &str, is_router: bool) -> Result<(), Box<dyn std::error::Error>> {
        
        for interface in &self.interfaces {
            if interface.r#type != link {
                continue;
            }

            let address = interface.ipv4.address.parse::<std::net::Ipv4Addr>()?;

            let netmask = if is_router {
                interface.ipv4.netmask.parse::<std::net::Ipv4Addr>()?
            } else {
                "255.0.0.0".parse::<std::net::Ipv4Addr>()?
            };

            let network = Network(ipnetwork::Ipv4Network::with_netmask(address, netmask)?);

            let address = match network.router(true) {
                Err(_) if is_router => {
                    eprintln!("Could not ping gateway, should setup router.");
                    network.router(false)?
                },
                Ok(addr) if is_router => {
                    eprintln!("Router is pinging at: {}, we must have the address set up and running.", addr);
                    // println!("My address is: {}", address);
                    // println!("Router address is: {}", addr);
                    // println!("Prefix is: {}", network.0.prefix());
                    
                    addr
                },
                _ => {
                    eprintln!("Could not ping router, will setup local routes and try again.");
                    address
                }
            };

            network.add_address(link, address).await?;
            
            repeat_until_ok!(network.router(true))?;

            println!("Network is: {:#?}", network);
        }

        Ok(())
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut plugin_manager = plugin_interface::PluginManager::default();

    unsafe {
        let lib = libloading::Library::new("/root/discovery_lifesigns/target/release/libdsc_lifesigns.so")?;
        let get_service: libloading::Symbol<extern "Rust" fn() -> Box<dyn plugin_interface::PluginService + Send>> = lib.get(b"get_service")?;

        let plugin = get_service();
        let (name, router_annon, srv_annon, _, _, _) = plugin.ehlo();
        
        println!("Registered plugin: {}", name);

        plugin_manager.register(name.into(), plugin);
    }


    let metadata = repeat_until_ok!(minreq::get("http://169.254.169.254/v1.json").send()
    .map_err(|err| format!("Failed fetching server metadata: {}", err)))?
    .json::<Metadata>()?;

    println!("Successfully fetched metadata: {:#?}", metadata);

    let mut routers = {
        // Connect to Maistroü        
        // let mut client = NodeMaistrouClient::connect("http://108.61.178.194:50051").await
        // .map_err(|err| format!("Failed connecting to Maistrou: {}", err))?;
        // This is will repeat the call at 5 seconds interval (5000ms)
        let mut client = repeat_until_ok!(NodeMaistrouClient::connect("http://108.61.178.194:50051").await
        .map_err(|err| format!("Failed connecting to Maistrou: {}", err)))?;

        eprintln!("Successfully connected to Maistrou");
        // std::thread::sleep_ms(10000);
        let mut routers = Vec::new();

        let mut page = 1;

        while page > 0 {            
            let response = repeat_until_ok!(
                client.list_nodes(
                    tonic::Request::new(
                        PageRequest {
                            perpage: 10,
                            page,
                            pending:
                            false,
                            r#type: 1
                        }
                    )
                ).await
            )?.into_inner();

            let mut nodes: Vec<node_funcs::Node> = serde_json::from_str(&response.data)?;

            routers.append(&mut nodes);

            if response.pages == 0 {
                break;
            }

            page += 1;
        }

        routers
    };

    let hostname = hostname::get()?;
    
    if hostname.to_str().ok_or("Failed converting hostname os string to string")? != metadata.hostname {
        hostname::set(&metadata.hostname)?;
        std::process::Command::new("hostnamectl")
        .args(["set-hostname", &metadata.hostname])
        .status()?;
    }

    // This should be some kind of an ENV or API call to 
    // Maistrou master, to check if this server is supposed
    // to be a router, node, or a backup
    let mut is_router = false;

    // Iterate through all routers and if some matches our
    // hostname, we should be a router too;
    // we will also remove ourselves from the list, so that 
    // we are only left with the peers
    if let Some(index) = routers.iter().position(|r| r.hostname == metadata.hostname) {                            
        println!("I should be configured as router ...");
        is_router = true;
        // Maybe we need to save the copy somewhere for later use
        routers.remove(index);
    }

    // Setup interfaces, we can have multiple
    // VPCs attached to single instance, hence 
    // why we need to iterate throug the metadata
    // set the master link for each interface
    // and up the interfaces + bridge
    // this will look for all interface vethX where X >= 1
    metadata.enable_priv_ifaces("private").await?;
    metadata.setup_priv_addresses("private", is_router).await?;
    
    // Only setup tunnel if I am router, all requests
    // should pass through me
    if is_router {
        // metadata.add_route("private", "10.0.0.0".parse()?, 8).await?;
        Network::add_route("private", "10.0.0.0".parse()?, 8, true).await?;
        metadata.setup_tunnel("private", routers).await?;
    }

    let addr = "0.0.0.0:50051".parse()?;
    let discovery = DiscoveryRPC::new(is_router, metadata)?;
    
    // Background task for router announcement
    let discovery_cloned = discovery.clone();
    tokio::spawn(async move { discovery_cloned.router_announcement().await });

    // Background task for router assessment
    // let discovery_cloned = discovery.clone();
    // tokio::spawn(async move { discovery_cloned.pinger().await });

    Server::builder()
    .add_service(DiscoveryServer::new(discovery))
    .serve(addr)
    .await?;


    return Ok(());

    eprintln!("IMPORTANT: WE SHOULD MANAGE `ip6tables` !!!!");
    eprintln!("IMPORTANT: WE SHOULD also `bridge fdb append 00:00:00:00:00:00 dev vxlan100 dst ${{remote server IPv6 address}}` !!!!");
    eprintln!("We should set tunnel links with other routers, fetching routers from Maistrou");

    Ok(())
}