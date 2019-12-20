//! IPFS node implementation
//#![deny(missing_docs)]

#[macro_use] extern crate failure;
#[macro_use] extern crate log;
pub use libp2p::PeerId;
use std::marker::PhantomData;
use std::path::PathBuf;
use futures::channel::mpsc::{channel, Sender, Receiver};
use std::future::Future;
use futures_util::future::FutureExt;
use libp2p::Swarm;

pub mod bitswap;
pub mod block;
mod config;
pub mod error;
pub mod ipld;
pub mod ipns;
pub mod p2p;
pub mod path;
pub mod repo;
pub mod unixfs;

pub use self::block::{Block, Cid};
use self::config::ConfigFile;
pub use self::error::Error;
use self::ipld::IpldDag;
pub use self::ipld::Ipld;
pub use self::p2p::{SwarmTypes, PubSubOut};
use self::p2p::{create_swarm, SwarmOptions, TSwarm, SwarmEvent};
pub use self::path::IpfsPath;
pub use self::repo::RepoTypes;
use self::repo::{create_repo, RepoOptions, Repo, RepoEvent};
use self::unixfs::File;
pub use libp2p::floodsub::{TopicBuilder, Topic, TopicHash};
pub use libp2p::Multiaddr;

static IPFS_LOG: &str = "info";
static IPFS_PATH: &str = ".rust-ipfs";
static XDG_APP_NAME: &str = "rust-ipfs";
static CONFIG_FILE: &str = "config.json";

/// All types can be changed at compile time by implementing
/// `IpfsTypes`.
pub trait IpfsTypes: SwarmTypes + RepoTypes {}
impl<T: RepoTypes> SwarmTypes for T {
    type TStrategy = bitswap::strategy::AltruisticStrategy<Self>;
}
impl<T: SwarmTypes + RepoTypes> IpfsTypes for T {}

/// Default IPFS types.
#[derive(Clone)]
pub struct Types;
impl RepoTypes for Types {
    type TBlockStore = repo::fs::FsBlockStore;
    #[cfg(feature = "rocksdb")]
    type TDataStore = repo::fs::RocksDataStore;
    #[cfg(not(feature = "rocksdb"))]
    type TDataStore = repo::mem::MemDataStore;
}

/// Testing IPFS types
#[derive(Clone)]
pub struct TestTypes;
impl RepoTypes for TestTypes {
    type TBlockStore = repo::mem::MemBlockStore;
    type TDataStore = repo::mem::MemDataStore;
}

/// Ipfs options
#[derive(Clone, Debug)]
pub struct IpfsOptions<Types: IpfsTypes> {
    _marker: PhantomData<Types>,
    /// The ipfs log level that should be passed to env_logger.
    pub ipfs_log: String,
    /// The path of the ipfs repo.
    pub ipfs_path: PathBuf,
    /// The ipfs config.
    pub config: ConfigFile,
}

impl Default for IpfsOptions<Types> {
    /// Create `IpfsOptions` from environment.
    fn default() -> Self {
        let ipfs_log = std::env::var("IPFS_LOG").unwrap_or(IPFS_LOG.into());
        let ipfs_path = std::env::var("IPFS_PATH").unwrap_or_else(|_| {
            let mut ipfs_path = std::env::var("HOME").unwrap_or("".into());
            ipfs_path.push_str("/");
            ipfs_path.push_str(IPFS_PATH);
            ipfs_path
        }).into();
        let xdg_dirs = xdg::BaseDirectories::with_prefix(XDG_APP_NAME).unwrap();
        let path = xdg_dirs.place_config_file(CONFIG_FILE).unwrap();
        let config = ConfigFile::new(path);

        IpfsOptions {
            _marker: PhantomData,
            ipfs_log,
            ipfs_path,
            config
        }
    }
}

impl Default for IpfsOptions<TestTypes> {
    /// Creates `IpfsOptions` for testing without reading or writing to the
    /// file system.
    fn default() -> Self {
        let ipfs_log = std::env::var("IPFS_LOG").unwrap_or(IPFS_LOG.into());
        let ipfs_path = std::env::var("IPFS_PATH").unwrap_or(IPFS_PATH.into()).into();
        let config = std::env::var("IPFS_TEST_CONFIG").map(|s| ConfigFile::new(s)).unwrap_or_else(|_| ConfigFile::default());
        IpfsOptions {
            _marker: PhantomData,
            ipfs_log,
            ipfs_path,
            config,
        }
    }
}

/// Ipfs struct creates a new IPFS node and is the main entry point
/// for interacting with IPFS.
#[derive(Clone)]
pub struct Ipfs<Types: IpfsTypes> {
    repo: Repo<Types>,
    dag: IpldDag<Types>,
    exit_events: Vec<Sender<IpfsEvent>>,
    swarm_events: async_std::sync::Sender<SwarmEvent>,
    pub pubsub_receiver: async_std::sync::Receiver<PubSubOut>,
}

enum IpfsEvent {
    Exit,
}

/// Configured Ipfs instace or value which can be only initialized.
pub struct UninitializedIpfs<Types: IpfsTypes> {
    repo: Repo<Types>,
    dag: IpldDag<Types>,
    moved_on_init: Option<(Receiver<RepoEvent>, TSwarm<Types>)>,
    exit_events: Vec<Sender<IpfsEvent>>,
}

impl<Types: IpfsTypes> UninitializedIpfs<Types> {
    pub fn new(options: IpfsOptions<Types>) -> Self {
        let repo_options = RepoOptions::<Types>::from(&options);
        let (repo, repo_events) = create_repo(repo_options);
        let swarm_options = SwarmOptions::<Types>::from(&options);
        let swarm = create_swarm(swarm_options, repo.clone());
        let dag = IpldDag::new(repo.clone());

        UninitializedIpfs {
            repo,
            dag,
            moved_on_init: Some((repo_events, swarm)),
            exit_events: Vec::default(),
        }
    }

    /// Initialize the ipfs repo.
    pub async fn start(mut self) -> Result<(Ipfs<Types>, impl std::future::Future<Output = ()>), Error> {
        use futures::compat::Stream01CompatExt;

        let (repo_events, swarm) = self.moved_on_init
            .take()
            .expect("Cant see how this should happen");

        self.repo.init().await?;

        let (sender, receiver) = channel::<IpfsEvent>(1);
        self.exit_events.push(sender);

        let (swarm_sender, swarm_receiver) = async_std::sync::channel::<SwarmEvent>(1000);
        let (pubsub_sender, pubsub_receiver) = async_std::sync::channel::<PubSubOut>(1000);


        let fut = IpfsFuture {
            swarm: swarm.compat(),
            repo_events,
            exit_events: receiver,
            swarm_events: swarm_receiver,
            pubsub_sender
        };
        

        let UninitializedIpfs { repo, dag, exit_events, .. } = self;

        Ok((Ipfs {
            repo,
            dag,
            exit_events,
            swarm_events: swarm_sender,
            pubsub_receiver,
        }, fut))
    }
}

impl<Types: IpfsTypes> Ipfs<Types> {
    /// Creates a new ipfs node.
    pub fn new(options: IpfsOptions<Types>) -> UninitializedIpfs<Types> {
        UninitializedIpfs::new(options)
    }

    /// Puts a block into the ipfs repo.
    pub async fn put_block(mut self, block: Block) -> Result<Cid, Error> {
        Ok(self.repo.put_block(block).await?)
    }

    /// Retrives a block from the ipfs repo.
    pub async fn get_block(mut self, cid: Cid) -> Result<Block, Error> {
        Ok(self.repo.get_block(&cid).await?)
    }

    /// Remove block from the ipfs repo.
    pub async fn remove_block(&mut self, cid: &Cid) -> Result<(), Error> {
        Ok(self.repo.remove_block(cid).await?)
    }

    /// Puts an ipld dag node into the ipfs repo.
    pub async fn put_dag(&self, ipld: Ipld) -> Result<IpfsPath, Error> {
        Ok(self.dag.put(ipld, cid::Codec::DagCBOR).await?)
    }

    /// Gets an ipld dag node from the ipfs repo.
    pub async fn get_dag(&self, path: IpfsPath) -> Result<Ipld, Error> {
        Ok(self.dag.get(path).await?)
    }

    /// Adds a file into the ipfs repo.
    pub async fn add(&self, path: PathBuf) -> Result<IpfsPath, Error> {
        let dag = self.dag.clone();
        let file = File::new(path).await?;
        let path = file.put_unixfs_v1(&dag).await?;
        Ok(path)
    }

    /// Gets a file from the ipfs repo.
    pub async fn get(&self, path: IpfsPath) -> Result<File, Error> {
        Ok(File::get_unixfs_v1(&self.dag, path).await?)
    }

    /// Exit daemon.
    pub fn exit_daemon(mut self) {
        for mut s in self.exit_events.drain(..) {
            let _ = s.try_send(IpfsEvent::Exit);
        }
    }
}

impl<Types: IpfsTypes> Ipfs<Types> {

    pub async fn dial_addr(&self, multiaddr: Multiaddr) {
        self.swarm_events.send(SwarmEvent::DialAddr(multiaddr)).await;
    }

    pub async fn dial(&self, peer_id: PeerId) {
        self.swarm_events.send(SwarmEvent::Dial(peer_id)).await
    }

    pub async fn add_external_address(&self, multiaddr: Multiaddr) {
        self.swarm_events.send(SwarmEvent::AddExternalAddress(multiaddr)).await
    }

    pub async fn ban_peer_id(&self, peer_id: PeerId) {
        self.swarm_events.send(SwarmEvent::BanPeerId(peer_id)).await
    }

    pub async fn unban_peer_id(&self, peer_id: PeerId) {
        self.swarm_events.send(SwarmEvent::UnbanPeerId(peer_id)).await
    }

    /// Subscribes to a topic.
    pub async fn subscribe(self, topic: Topic) {
        self.swarm_events.send(SwarmEvent::Subscribe(topic)).await
    }

    /// Unsubscribes from a topic.
    ///
    /// Note that this only requires a `TopicHash` and not a full `Topic`.
    pub async fn unsubscribe(self, topic: Topic) {
        self.swarm_events.send(SwarmEvent::Unsubscribe(topic)).await
    }

    /// Publishes a message to the network, if we're subscribed to the topic only.
    pub async fn publish(self, topic: TopicHash, data: Vec<u8>) {
        self.swarm_events.send(SwarmEvent::Publish{topic, data}).await
    }

    /// Publishes a message to the network, even if we're not subscribed to the topic.
    pub async fn publish_any(self, topic: TopicHash, data: Vec<u8>) {
        self.swarm_events.send(SwarmEvent::PublishAny{topic, data}).await
    }

    /// Publishes a message with multiple topics to the network.
    /// > **Note**: Doesn't do anything if we're not subscribed to any of the topics.
    pub async fn publish_many(self, topic: Vec<TopicHash>, data: Vec<u8>) {
        self.swarm_events.send(SwarmEvent::PublishMany{topic, data}).await
    }

    /// Publishes a message with multiple topics to the network, even if we're not subscribed to any of the topics.
    pub async fn publish_many_any(self, topic: Vec<libp2p::floodsub::TopicHash>, data: Vec<u8>) {
        self.swarm_events.send(SwarmEvent::PublishManyAny{topic, data}).await
    }
}

pub struct IpfsFuture<Types: SwarmTypes> {
    swarm: futures::compat::Compat01As03<TSwarm<Types>>,
    repo_events: Receiver<RepoEvent>,
    exit_events: Receiver<IpfsEvent>,
    swarm_events: async_std::sync::Receiver<SwarmEvent>,
    pubsub_sender: async_std::sync::Sender<PubSubOut>,
}

use std::pin::Pin;
use std::task::{Poll, Context};

impl<Types: SwarmTypes> Future for IpfsFuture<Types> {
    type Output = ();
    
    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        use futures::Stream;
        loop {
            // FIXME: this can probably be rewritten as a async { loop { select! { ... } } } once
            // libp2p uses std::future ... I couldn't figure out way to wrap it as compat,
            // box it and fuse it to for it to be used with futures::select!

            // {
            //     let pin = Pin::new(&mut self.exit_events);

            //     if let Poll::Ready(Some(IpfsEvent::Exit)) = pin.poll_next(ctx) {
            //         debug!("get some from exit_events");
            //         return Poll::Ready(());
            //     }
            // }
            loop {
                let pin = Pin::new(&mut self.swarm_events);
                match pin.poll_next(ctx) {
                    Poll::Ready(Some(SwarmEvent::DialAddr(multiaddr))) => 
                        Swarm::dial_addr(&mut self.swarm.get_mut(), multiaddr.clone()).expect("should work :-)"),
                    Poll::Ready(Some(SwarmEvent::Dial(peer_id))) => 
                        Swarm::dial(&mut self.swarm.get_mut(), peer_id.clone()),
                    Poll::Ready(Some(SwarmEvent::AddExternalAddress(multiaddr))) => 
                        Swarm::add_external_address(self.swarm.get_mut(), multiaddr),
                    Poll::Ready(Some(SwarmEvent::BanPeerId(peer_id))) => 
                        Swarm::ban_peer_id(self.swarm.get_mut(), peer_id),
                    Poll::Ready(Some(SwarmEvent::UnbanPeerId(peer_id))) => 
                        Swarm::unban_peer_id(self.swarm.get_mut(), peer_id),
                        
                    Poll::Ready(Some(SwarmEvent::Subscribe(t))) => {
                        self.swarm.get_mut().floodsub.subscribe(t);
                    }, 
                    Poll::Ready(Some(SwarmEvent::Unsubscribe(t))) => {
                        self.swarm.get_mut().floodsub.unsubscribe(t);
                    }, 
                    Poll::Ready(Some(SwarmEvent::Publish{topic, data})) => 
                        self.swarm.get_mut().floodsub.publish(topic, data), 
                    Poll::Ready(Some(SwarmEvent::PublishAny{topic, data})) => 
                        self.swarm.get_mut().floodsub.publish_any(topic, data), 
                    Poll::Ready(Some(SwarmEvent::PublishMany{topic, data})) => 
                        self.swarm.get_mut().floodsub.publish_many(topic, data), 
                    Poll::Ready(Some(SwarmEvent::PublishManyAny{topic, data})) => 
                        self.swarm.get_mut().floodsub.publish_many_any(topic, data),  
                    Poll::Ready(None) => panic!("swarm_events should never be closed?"),
                    Poll::Pending => break,
                }
            }

            {
                loop {
                    let pin = Pin::new(&mut self.repo_events);
                    match pin.poll_next(ctx) {
                        Poll::Ready(Some(RepoEvent::WantBlock(cid))) =>
                            self.swarm.get_mut().want_block(cid),
                        Poll::Ready(Some(RepoEvent::ProvideBlock(cid))) =>
                            self.swarm.get_mut().provide_block(cid),
                        Poll::Ready(Some(RepoEvent::UnprovideBlock(cid))) =>
                            self.swarm.get_mut().stop_providing_block(&cid),
                        Poll::Ready(None) => panic!("other side closed the repo_events?"),
                        Poll::Pending => break,
                    }
                }
            }

            {
                let poll = Pin::new(&mut self.swarm).poll_next(ctx);
                match poll {
                    Poll::Ready(Some(Ok(msg @ PubSubOut::FloodsubMessage{ .. }))) => {
                        let _unused = self.pubsub_sender.send(msg).boxed().as_mut().poll(ctx);
                    },
                    Poll::Ready(Some(_)) => {},
                    Poll::Ready(None) => { return Poll::Ready(()); },
                    Poll::Pending => { return Poll::Pending; }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::{FutureExt, TryFutureExt};

    /// Testing helper for std::future::Futures until we can upgrade tokio
    pub(crate) fn async_test<O, F>(future: F) -> O
        where O: 'static + Send,
              F: std::future::Future<Output = O> + 'static + Send
    {
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::run(async move {
            let tx = tx;
            let awaited = future.await;
            tx.send(awaited).unwrap();
        }.unit_error().boxed().compat());
        rx.recv().unwrap()
    }

    #[test]
    fn test_put_and_get_block() {
        async_test(async move {
            let options = IpfsOptions::<TestTypes>::default();
            let block = Block::from("hello block\n");
            let ipfs = Ipfs::new(options);
            let (mut ipfs, fut) = ipfs.start().await.unwrap();
            tokio::spawn(fut.unit_error().boxed().compat());

            let cid: Cid = ipfs.put_block(block.clone()).await.unwrap();
            let new_block = ipfs.get_block(&cid).await.unwrap();
            assert_eq!(block, new_block);

            ipfs.exit_daemon();
        });
    }

    #[test]
    fn test_put_and_get_dag() {
        let options = IpfsOptions::<TestTypes>::default();

        async_test(async move {

            let (ipfs, fut) = Ipfs::new(options).start().await.unwrap();
            tokio::spawn(fut.unit_error().boxed().compat());

            let data: Ipld = vec![-1, -2, -3].into();
            let cid = ipfs.put_dag(data.clone()).await.unwrap();
            let new_data = ipfs.get_dag(cid.into()).await.unwrap();
            assert_eq!(data, new_data);

            ipfs.exit_daemon();
        });
    }
}
