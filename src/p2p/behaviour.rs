use crate::bitswap::{Bitswap, Strategy};
use crate::block::Cid;
use crate::p2p::{SwarmOptions, SwarmTypes};
use crate::repo::Repo;
use libp2p::{NetworkBehaviour};
use libp2p::swarm::{NetworkBehaviourEventProcess, NetworkBehaviourAction};
use libp2p::core::muxing::{StreamMuxerBox, SubstreamRef};
use libp2p::identify::{Identify, IdentifyEvent};
use libp2p::mdns::{Mdns, MdnsEvent};
use libp2p::ping::{Ping, PingEvent};
use libp2p::{Multiaddr, PeerId};
use libp2p::floodsub::{Floodsub, FloodsubEvent, TopicHash, Topic};
//use parity_multihash::Multihash;
use std::sync::Arc;
use tokio::prelude::*;


/// Behaviour type.
#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourOut", poll_method = "poll")]
pub struct Behaviour<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> {
    pub mdns: Mdns<TSubstream>,
    pub bitswap: Bitswap<TSubstream, TSwarmTypes>,
    pub ping: Ping<TSubstream>,
    pub identify: Identify<TSubstream>,
    pub floodsub: Floodsub<TSubstream>,
    
    /// Queue of events to produce for the outside.
    #[behaviour(ignore)]
    events: Vec<BehaviourOut>,
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

/// Event that can be emitted by the behaviour.
#[derive(Debug)]
pub enum BehaviourOut {
    FloodsubMessage{
        topics: Vec<TopicHash>, 
        data: Vec<u8>, 
        source: PeerId, 
        sequent_number: Vec<u8>,
    },
}

#[derive(Clone, Debug)]
pub enum SwarmEvent {
    DialAddr(Multiaddr),
    Dial(PeerId),
    AddExternalAddress(Multiaddr),
    BanPeerId(PeerId),
    UnbanPeerId(PeerId),
    Subscribe(Topic),
    Unsubscribe(Topic),
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
                debug!("get message: {} {:?} from {}", String::from_utf8(m.data.clone()).unwrap(), m.topics, m.source);
                self.events.push(BehaviourOut::FloodsubMessage{
                    topics: m.topics,
                    data: m.data,
                    source: m.source,
                    sequent_number: m.sequence_number,
                });
            },
            _ => {}

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
        let floodsub = Floodsub::new(options.peer_id.to_owned());

        Behaviour {
            mdns,
            bitswap,
            ping,
            identify,
            floodsub,
			events: Vec::new(),
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

impl<TSubstream: AsyncRead + AsyncWrite, TSwarmTypes: SwarmTypes> Behaviour<TSubstream, TSwarmTypes> {
	fn poll<TEvent>(&mut self) -> Async<NetworkBehaviourAction<TEvent, BehaviourOut>> {
		if !self.events.is_empty() {
			return Async::Ready(NetworkBehaviourAction::GenerateEvent(self.events.remove(0)))
		}

		Async::NotReady
	}
}

/// Behaviour type.
pub(crate) type TBehaviour<TSwarmTypes> = Behaviour<SubstreamRef<Arc<StreamMuxerBox>>, TSwarmTypes>;

/// Create a IPFS behaviour with the IPFS bootstrap nodes.
pub fn build_behaviour<TSwarmTypes: SwarmTypes>(options: SwarmOptions<TSwarmTypes>, repo: Repo<TSwarmTypes>) -> TBehaviour<TSwarmTypes> {
    info!("Behaviour::new {}", options.peer_id);
    Behaviour::new(options, repo)
}
