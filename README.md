<!--
https://superuser.com/questions/757782/iptables-append-forward-required-for-routing-between-nic-alias-ips
https://joejulian.name/post/how-to-configure-linux-vxlans-with-multiple-unicast-endpoints/
https://lxadm.com/Unicast_VXLAN:_overlay_network_for_multiple_servers_with_dozens_of_containers#MTU_issue
https://briantsaunders.github.io/posts/2019/05/creating-vxlan-tunnel-in-linux-with-python/
https://baturin.org/docs/iproute2/#ip-link-vxlan
https://backreference.org/2013/07/23/gre-bridging-ipsec-and-nfqueue/ ***
https://superuser.com/questions/1185070/transparent-tunnel-between-interfaces-on-remote-hosts
https://vincent.bernat.ch/en/blog/2017-vxlan-linux#other-considerations
https://vincent.bernat.ch/en/blog/2012-multicast-vxlan
 -->

 # Discovery
 This is a service which should start up right after server bootup and network setup. It plays the role of a mesh service discovery, while not in the exact sense. It contacts remote API and gets a list of servers - routers as well as metadata for the server it is suited on.

 ## Features
 - UPs private interfaces and creates routes and addresses
 - Looks up remote router servers and setups an overlay network link between them (via IPv6)
 - (Should) Makes sures that servers inter-DC are connected at all times

## To do
- Configuration file parser, so we can avoid hardcoded settings
- Webhooks
- Docs??