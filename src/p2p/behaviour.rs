use crate::bitswap::{Bitswap, Strategy};
use crate::block::Cid;
use crate::p2p::{SwarmOptions, SwarmTypes};
use crate::repo::Repo;
use libp2p::{NetworkBehaviour};
use libp2p::swarm::NetworkBehaviourEventProcess;
use libp2p::core::muxing::{StreamMuxerBox, SubstreamRef};
use libp2p::identify::{Identify, IdentifyEvent};
use libp2p::mdns::{Mdns, MdnsEvent};
use libp2p::ping::{Ping, PingEvent};
use libp2p::{Multiaddr, PeerId};
use libp2p::floodsub::{Floodsub, FloodsubEvent, TopicBuilder, TopicHash, Topic};
//use parity_multihash::Multihash;
use std::sync::Arc;
use tokio::prelude::*;


/// Behaviour type.
#[derive(NetworkBehaviour)]
pub struct Behaviour<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> {
    mdns: Mdns<TSubstream>,
    bitswap: Bitswap<TSubstream, TSwarmTypes>,
    ping: Ping<TSubstream>,
    identify: Identify<TSubstream>,
    floodsub: Floodsub<TSubstream>,
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<MdnsEvent> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(list) => {
                for (peer, _) in list {
                    debug!("mdns: Discovered peer {}", peer.to_base58());
                    self.bitswap.connect(peer.clone());
                    self.floodsub.add_node_to_partial_view(peer);
                }
            }
            MdnsEvent::Expired(list) => {
                for (peer, _) in list {
                    if !self.mdns.has_node(&peer) {
                        info!("mdns: Expired peer {}", peer.to_base58());
                        self.floodsub.remove_node_from_partial_view(&peer);
                    }
                }
            }
        }
    }
}


#[derive(Clone, Debug)]
pub enum SwarmEvent {
    DialAddr(Multiaddr),
    Dial(PeerId),
    AddExternalAddress(Multiaddr),
    BanPeerId(PeerId),
    UnbanPeerId(PeerId),
    Subscribe (Topic),
    Unsubscribe (Topic),
    Publish {
        topic: TopicHash,
        data: Vec<u8>,
    },
    PublishAny {
        topic: TopicHash,
        data: Vec<u8>,
    },
    PublishMany {
        topic: Vec<TopicHash>,
        data: Vec<u8>,
    },
    PublishManyAny {
        topic: Vec<TopicHash>,
        data: Vec<u8>,
    },
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<()> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, _event: ()) {
    }
}
impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<SwarmEvent> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, _event: SwarmEvent) {
    }
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<PingEvent> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, event: PingEvent) {
        use libp2p::ping::handler::{PingSuccess, PingFailure};
        match event {
            PingEvent { peer, result: Result::Ok(PingSuccess::Ping { rtt }) } => {
                debug!("ping: rtt to {} is {} ms", peer.to_base58(), rtt.as_millis());
            },
            PingEvent { peer, result: Result::Ok(PingSuccess::Pong) } => {
                debug!("ping: pong from {}", peer.to_base58());
            },
            PingEvent { peer, result: Result::Err(PingFailure::Timeout) } => {
                warn!("ping: timeout to {}", peer.to_base58());
            },
            PingEvent { peer, result: Result::Err(PingFailure::Other { error }) } => {
                error!("ping: failure with {}: {}", peer.to_base58(), error);
            }
        }
    }
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<IdentifyEvent> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, event: IdentifyEvent) {
        match event {
            IdentifyEvent::Received{ peer_id, .. } => info!("Received: {}", peer_id),
            IdentifyEvent::Error{ peer_id, .. } => info!("error: {}", peer_id),
            _ => ()
        }
    }
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes>
    NetworkBehaviourEventProcess<FloodsubEvent> for
    Behaviour<TSubstream, TSwarmTypes>
{
    fn inject_event(&mut self, event: FloodsubEvent) {
        match event {
            FloodsubEvent::Message(m) =>  {
                info!("get message: {} {:?}", String::from_utf8(m.data).unwrap(), m.topics);
            },
            FloodsubEvent::Subscribed{ .. } => {
                //let topic_resp = TopicBuilder::new("hello").build();
                //let msg = format!("I'm also here\n++ from code to {} ++\n", topic.clone().into_string());
                //self.floodsub.publish(topic_resp, msg)
            },
            FloodsubEvent::Unsubscribed{ .. } => {}

        }
    }
}

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> Behaviour<TSubstream, TSwarmTypes>
{
    pub fn new(options: SwarmOptions<TSwarmTypes>, repo: Repo<TSwarmTypes>) -> Self {
        info!("Local peer id: {}", options.peer_id.to_base58());

        let mdns = Mdns::new().expect("Failed to create mDNS service");
        let strategy = TSwarmTypes::TStrategy::new(repo);
        let bitswap = Bitswap::new(strategy);
        let ping = Ping::default();
        let identify = Identify::new(
            "/ipfs/0.1.0".into(),
            "rust-ipfs".into(),
            options.key_pair.public(),
        );
        let mut floodsub = Floodsub::new(options.peer_id.to_owned());

        let topic = TopicBuilder::new("hello").build();
        let res = floodsub.subscribe(topic.clone());

        info!("can subscribe {}", res);
        
        Behaviour {
            mdns,
            bitswap,
            ping,
            identify,
            floodsub,
        }
    }

    pub fn want_block(&mut self, cid: Cid) {
        info!("Want block {}", cid.to_string());
        self.bitswap.want_block(cid, 1);
    }

    pub fn provide_block(&mut self, cid: Cid) {
        info!("Providing block {}", cid.to_string());
    }

    pub fn stop_providing_block(&mut self, cid: &Cid) {
        info!("Finished providing block {}", cid.to_string());
    }
}

/// Behaviour type.
pub(crate) type TBehaviour<TSwarmTypes> = Behaviour<SubstreamRef<Arc<StreamMuxerBox>>, TSwarmTypes>;

/// Create a IPFS behaviour with the IPFS bootstrap nodes.
pub fn build_behaviour<TSwarmTypes: SwarmTypes>(options: SwarmOptions<TSwarmTypes>, repo: Repo<TSwarmTypes>) -> TBehaviour<TSwarmTypes> {
    info!("Behaviour::new {}", options.peer_id);
    Behaviour::new(options, repo)
}


impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> Behaviour<TSubstream, TSwarmTypes> {
    /// Subscribes to a topic.
    ///
    /// Returns true if the subscription worked. Returns false if we were already subscribed.
    pub fn subscribe(&mut self, topic: Topic) -> bool {
        self.floodsub.subscribe(topic)
    }

    /// Unsubscribes from a topic.
    ///
    /// Note that this only requires a `TopicHash` and not a full `Topic`.
    ///
    /// Returns true if we were subscribed to this topic.
    pub fn unsubscribe(&mut self, topic: Topic) -> bool {
        self.floodsub.unsubscribe(topic)
    }

    /// Publishes a message to the network, if we're subscribed to the topic only.
    pub fn publish(&mut self, topic: TopicHash, data: impl Into<Vec<u8>>) {
        self.floodsub.publish(topic, data)
    }

    /// Publishes a message to the network, even if we're not subscribed to the topic.
    pub fn publish_any(&mut self, topic: TopicHash, data: impl Into<Vec<u8>>) {
        self.floodsub.publish_any(topic, data)
    }

    /// Publishes a message with multiple topics to the network.
    /// > **Note**: Doesn't do anything if we're not subscribed to any of the topics.
    pub fn publish_many(&mut self, topic: impl IntoIterator<Item = TopicHash>, data: impl Into<Vec<u8>>) {
        self.floodsub.publish_many(topic, data)
    }

    /// Publishes a message with multiple topics to the network, even if we're not subscribed to any of the topics.
    pub fn publish_many_any(&mut self, topic: impl IntoIterator<Item = TopicHash>, data: impl Into<Vec<u8>>) {
        self.floodsub.publish_many_any(topic, data)
    }
}

// impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> Behaviour<TSubstream, TSwarmTypes> { {
//     pub fn dial(&mut self, peer_id: PeerId) {
//         Swarm::dial(self, peer_id.clone()),
//     }
//     pub fn add_external_address(&mut self, multiaddr: Multiaddr) {
//         Swarm::add_external_address(self, multiaddr),
//     }
//     pub fn ban_peer_id(&mut self, peer_id: PeerId) {
//         Swarm::ban_peer_id(self, peer_id),
//     }
//     pub fn unban_peer_id(&mut self, peer_id: PeerId) {
//         Swarm::unban_peer_id(self, peer_id),
//     }
// }