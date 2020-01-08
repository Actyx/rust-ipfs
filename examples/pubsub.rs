use futures::{FutureExt, TryFutureExt};

use ipfs::{Ipfs, IpfsOptions, TestTypes, TopicBuilder, TopicHash, PubSubOut};
use async_std::{io, task};
use futures::{future, prelude::*};
use futures::task::{Poll, Context};

use ipfs::block::{Block, Cid};
use libp2p::kad::{Quorum, Record, {record::Key}};

fn main() {
  let options = IpfsOptions::<TestTypes>::default();
  env_logger::Builder::new().parse_filters(&options.ipfs_log).init();


  let prefix = cid::Prefix {
    version: cid::Version::V1,
    codec: cid::Codec::DagCBOR,
    mh_type: multihash::Hash::SHA2256,
    mh_len: 32,
  };
  let data = b"test12342134234";
  let cid = cid::Cid::new_from_prefix(&prefix, data);
  let block = Block::new(data.to_vec(), cid);

  task::block_on(async move {
    let (ipfs, fut) = Ipfs::new(options).await.start().await.unwrap();
    task::spawn(fut);

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let topic = TopicBuilder::new("hello").build();

    let cid = ipfs.clone().put_block(block).await;
    println!("{}", cid.unwrap());

    ipfs.clone().subscribe(topic.clone()).await;
    let r1 = ipfs.pubsub_receiver.clone();
    task::spawn(async move {
      loop {
        if let Some(PubSubOut::FloodsubMessage{ data, ..}) = r1.recv().await {
          println!("get message {:?}", String::from_utf8(data).unwrap());
        }
      }
    });
    let topic = TopicBuilder::new("hello").build();
    let ipfs_cp = ipfs.clone();
    // wait for peer connect
    // let _task = Delay::new(Instant::now() + Duration::from_millis(1000)).compat().await.unwrap();
    ipfs.clone().publish(topic.clone().into(), Vec::from("hallo this is my first message\n")).await;

    let key = Key::new(&"hello".to_owned());
    let value = b"world";
    let record = Record::new(key.clone(), value.to_vec());
    let quorum = Quorum::One;    
    ipfs.clone().put_record(record, quorum).await;
    ipfs.clone().get_record(key.clone(), quorum).await;
    let key2 = Cid::from("QmWnJatWH6XneAbi2bVJotC1zTsvm4aF78asvbaEPhwb4i").unwrap();
    ipfs.get_providers(Key::new(&key2.to_bytes())).await;

    task::block_on(future::poll_fn(move |cx: &mut Context| {
      loop {
        match stdin.poll_next_unpin(cx) {
            Poll::Ready(Some(line)) => {
              let line: String = line.unwrap();
              let parts: Vec<&str> = line.split_ascii_whitespace().collect();
              match parts.as_slice() {
                &["pub", msg] => { 
                  task::spawn(ipfs_cp.clone().publish_any(topic.clone().into(), Vec::from(msg)));
                },
                &["dht", "put", key, value] => {
                  let key = Key::new(&key.to_owned());
                  let record = Record::new(key.clone(), value.as_bytes().to_vec());
                  task::spawn(ipfs_cp.clone().put_record(record, quorum));
                }
                &["dht", "get", key] => {
                  let key = Key::new(&key.to_owned());
                  task::spawn(ipfs_cp.clone().get_record(key, quorum));
                },

                &["block", "put", text] => {
                  let cid = cid::Cid::new_from_prefix(&prefix, text.as_bytes());
                  let block = Block::new(data.to_vec(), cid);
                  task::spawn(ipfs_cp.clone().put_block(block).map(|cid| {
                    println!("{}", cid.unwrap());
                  }));
                },

                &["block", "get", key] => {
                  let cid = cid::Cid::from(key).unwrap();
                  println!("{}", cid);
                  task::spawn(ipfs_cp.clone().get_block(cid).map_ok(|x| println!("{:?}", x)));
                }
                _ => {
                  task::spawn(ipfs_cp.clone().publish_any(topic.clone().into(), Vec::from(line)));
                }
              }
            }
            Poll::Ready(None) => return Poll::Ready(()),
            Poll::Pending => break,
        };
      }
      Poll::Pending
    }));


    loop {
      // let _task = Delay::new(Instant::now() + Duration::from_millis(1000)).compat().await.unwrap();
    }
  })
}
