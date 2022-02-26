use fastping_rs::Pinger;
use fastping_rs::PingResult::Receive;

use futures::stream::TryStreamExt;

#[derive(Debug, Clone)]
pub struct Network(pub ipnetwork::Ipv4Network);

impl Network {
    pub fn addr(&self) -> std::net::Ipv4Addr {
        self.0.ip()
    }

    pub fn network(&self) -> std::net::Ipv4Addr {
        self.0.network()
    }

    pub fn gateway(&self) -> Result<std::net::Ipv4Addr, Box<dyn std::error::Error>> {
        self.0.nth(1).ok_or::<Box <dyn std::error::Error>>("Failed fetching router".into())
    }

    pub fn prefix(&self) -> u8 {
        self.0.prefix()
    }

    pub fn router(&self, check: bool) -> Result<std::net::Ipv4Addr, Box<dyn std::error::Error>> {
        let router = self.0.nth(1).ok_or::<Box <dyn std::error::Error>>("Failed fetching router".into())?;

        if check {
            ping(router)?;
        }

        Ok(router)
    }

    async fn check_addr_route(&self, handle: &rtnetlink::Handle, address: std::net::Ipv4Addr, link_iface: u32, no_pref_source: bool) -> Result<(), Box<dyn std::error::Error>> {

        let mut routes = handle.route().get(rtnetlink::IpVersion::V4).execute();

        while let Some(route) = routes.try_next().await? {
            if let None = route.nlas.iter().position(|n| n == &rtnetlink::packet::rtnl::nlas::route::Nla::Oif(link_iface)) {
                continue;
            }

            if !no_pref_source {
                if let None = route.nlas.iter().position(|n| n == &rtnetlink::packet::rtnl::nlas::route::Nla::PrefSource(address.octets().to_vec())) {
                    continue;
                }
            }
            
            if let None = route.nlas.iter().position(|n| n == &rtnetlink::packet::rtnl::nlas::route::Nla::Destination(self.0.network().octets().to_vec())) {
                continue;
            }
            
            println!("Route is: {:?}", route);

            if route.header.destination_prefix_length == self.0.prefix() {
                eprintln!("{}/{} route exists", address, self.0.prefix());
                return Ok(());
            }            
        }

        Err(format!("{}/{} route does not exist", address, self.0.prefix()).into())
    }

    pub async fn add_route(link_iface: &str, address: std::net::Ipv4Addr, prefix: u8, no_pref_source: bool) -> Result<(), Box<dyn std::error::Error>> {

        let (connection, handle, _) = rtnetlink::new_connection()?;

        let network = Network(ipnetwork::Ipv4Network::new(address, prefix)?);

        tokio::spawn(connection);

        let mut private_link = handle.link().get().match_name(link_iface.into()).execute();

        if let Some(link) = private_link.try_next().await? {
            // We do not have a route
            if let Err(err) = network.check_addr_route(&handle, address, link.header.index, no_pref_source).await {
                eprintln!("{}", err);

                // Add a route
                network.add_addr_route(&handle, address, link.header.index, no_pref_source).await?;
                
                // Initiate route check once again to make sure
                network.check_addr_route(&handle, address, link.header.index, no_pref_source).await?;

                eprintln!("Route successfully created");
            }

            return Ok(())
        }

        Ok(())
    }

    async fn add_addr_route(&self, handle: &rtnetlink::Handle, address: std::net::Ipv4Addr, link_iface: u32, no_pref_source: bool) -> Result<(), Box<dyn std::error::Error>> {
        // println!("We will add Address maybe?: {}", address);
        // println!("We will add PrefSource: {}", self.0.ip());
        // println!("We will add Destination IP: {}", self.0.network());
        // println!("We will add Destionation Prefix: {}", self.0.prefix());

        // output interface = bridge
        // let table = 254 (main table);
        let proto = 4;
        let scope = 253;

        // Add IPv4 route
        let mut route = handle.route().add().v4()
        .output_interface(link_iface)
        .scope(scope)
        .protocol(proto)
        .destination_prefix(self.0.network(), self.0.prefix());

        if !no_pref_source {
            route = route.pref_source(address);
        }

        route.execute().await
        .map_err(|err| format!("Failed adding route: {}/{} dev {}: {:?}", self.0.network(), self.0.prefix(), link_iface, err))?;

        Ok(())
    }

    pub async fn add_address(&self, link_iface: &str, address: std::net::Ipv4Addr) -> Result<(), Box<dyn std::error::Error>> {
        
        let (connection, handle, _) = rtnetlink::new_connection()?;

        tokio::spawn(connection);

        let mut private_link = handle.link().get().match_name(link_iface.into()).execute();

        if let Some(link) = private_link.try_next().await? {
            // We need to check first if the address exists
            let mut addresses = handle.address().get().set_link_index_filter(link.header.index).execute();

            while let Some(_address) = addresses.try_next().await? {
                if let None = _address.nlas.iter().position(|n| n == &rtnetlink::packet::rtnl::nlas::address::Nla::Address(address.octets().to_vec())) {
                    continue;
                }
                
                eprintln!("Address {} already exists", address);
                
                // We do not have a route
                if let Err(err) = self.check_addr_route(&handle, address, link.header.index, false).await {
                    eprintln!("{}", err);

                    // Add a route
                    self.add_addr_route(&handle, address, link.header.index, false).await?;
                    
                    // Initiate route check once again to make sure
                    self.check_addr_route(&handle, address, link.header.index, false).await?;

                    eprintln!("Route successfully created");
                }

                return Ok(())
            }

            handle.address().add(link.header.index, address.into(), self.0.prefix()).execute().await
            .map_err(|err| format!("Failed creating address: {} for iface: {}: {}", address, link_iface, err))?;
        }

        Ok(())
    }
}

pub fn ping(address: std::net::Ipv4Addr) -> Result<(), Box<dyn std::error::Error>> {
    let (pinger, res) = Pinger::new(None, None)?;

    pinger.add_ipaddr(&address.to_string());
    pinger.run_pinger();

    let mut seq = 0;

    while seq < 2 {
        match res.recv() {
            Ok(res) => {
                match res {
                    // Idle { addr: _ } => {
                    //     // eprintln!("No answer from {} retrying (max = 5, seq = {})", addr, seq);
                    //     eprintln!(".");
                    // },
                    Receive { .. } => {
                        // println!("Answer from {} seq={} rtt={:?}", addr, seq, rtt);
                        return Ok(());
                    },
                    _ => {}
                }
            }, 
            Err(err) => return Err(err.to_string().into())
        }

        seq += 1;
    }

    Err(format!("Could not find router at {}", address).into())
}