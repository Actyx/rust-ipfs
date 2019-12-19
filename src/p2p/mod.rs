//! P2P handling for IPFS nodes.
use crate::bitswap::Strategy;
use crate::IpfsOptions;
use crate::repo::{Repo, RepoTypes};
use libp2p::{Multiaddr, PeerId};
use libp2p::Swarm;
use libp2p::identity::Keypair;
use std::marker::PhantomData;
use crossbeam_channel::{bounded, Receiver, Sender};

mod behaviour;
mod transport;
pub use behaviour::{SwarmEvent};

pub type TSwarm<SwarmTypes> = Swarm<transport::TTransport, behaviour::TBehaviour<SwarmTypes>>;

pub trait SwarmTypes: RepoTypes + Sized {
    type TStrategy: Strategy<Self>;
}

#[derive(Clone)]
pub struct SwarmOptions<TSwarmTypes: SwarmTypes> {
    _marker: PhantomData<TSwarmTypes>,
    pub key_pair: Keypair,
    pub peer_id: PeerId,
    pub bootstrap: Vec<(Multiaddr, PeerId)>,
}

impl<TSwarmTypes: SwarmTypes> From<&IpfsOptions<TSwarmTypes>> for SwarmOptions<TSwarmTypes> {
    fn from(options: &IpfsOptions<TSwarmTypes>) -> Self {
        let key_pair = options.config.secio_key_pair();
        let peer_id = key_pair.public().into_peer_id();
        let bootstrap = options.config.bootstrap();
        SwarmOptions {
            _marker: PhantomData,
            key_pair,
            peer_id,
            bootstrap,
        }
    }
}

pub struct IpfsSwarm<TSwarmTypes: SwarmTypes> {
    pub swarm: TSwarm<TSwarmTypes>,
    pub swarm_emit: Sender<SwarmEvent>,
    pub swarm_events: Receiver<SwarmEvent>,
}
/// Creates a new IPFS swarm.
pub fn create_swarm<TSwarmTypes: SwarmTypes>(options: SwarmOptions<TSwarmTypes>, repo: Repo<TSwarmTypes>) -> IpfsSwarm<TSwarmTypes> {
    let peer_id = options.peer_id.clone();

    // Set up an encrypted TCP transport over the Mplex protocol.
    let transport = transport::build_transport(&options);

    // Create a Kademlia behaviour
    let behaviour = behaviour::build_behaviour(options.clone(), repo);
    
    // Create a Swarm
    let mut swarm = libp2p::Swarm::new(transport, behaviour, peer_id);
    
    // let topic = TopicBuilder::new("hello").build();
    // swarm.behaviour.subscribe(topic);

    // Listen on all interfaces and whatever port the OS assigns
    let listener_id = Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse().unwrap()).unwrap();
    info!("ListenerId {:?}", listener_id);

    let (swarm_sender, swarm_receiver) = bounded(10000);

    // let _unused = options.bootstrap.iter().map(|node| {
    //     println!("connect to {:?}" , node.0);
    //     Swarm::dial_addr(&mut swarm, node.0.clone()).unwrap();
    // }).collect::<()>();

    IpfsSwarm {
        swarm,
        swarm_emit: swarm_sender,
        swarm_events: swarm_receiver,
    }
}
