use futures::{FutureExt, TryFutureExt};

use ipfs::{Ipfs, IpfsOptions, TestTypes, TopicBuilder, TopicHash, PubSubOut};
use libp2p::{tokio_codec::{FramedRead, LinesCodec}};
use tokio::timer::Delay;

use std::time::{Duration, Instant};
use futures_util::compat::Future01CompatExt;
use futures01::{Stream as Stream01, Future as Future01, Async};

use async_std::task;

fn main() {
  let options = IpfsOptions::<TestTypes>::default();
  env_logger::Builder::new().parse_filters(&options.ipfs_log).init();

  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on_all(
      async move {
        let (ipfs, fut) = Ipfs::new(options).start().await.unwrap();
        tokio::spawn(fut.unit_error().boxed().compat());

        let stdin = tokio_stdin_stdout::stdin(0);
        let mut framed_stdin = FramedRead::new(stdin, LinesCodec::new());

        let ipfs_cp = ipfs.clone();
        tokio::spawn(futures01::future::poll_fn(move || {
          loop {
            match framed_stdin.poll().expect("Error while polling stdin") {
                Async::Ready(Some(line)) => {
                  let topic = TopicHash::from(TopicBuilder::new("hello").build());
                  // one poll is enough for demo
                  let _unused = ipfs_cp.clone().publish(topic, Vec::from(line)).unit_error().boxed().compat().poll();
                }
                Async::Ready(None) => panic!("Stdin closed"),
                Async::NotReady => break,
            };
          }
          Ok(Async::NotReady)
        }));



        let r1 = ipfs.pubsub_receiver.clone();
        task::spawn(async move {
          loop {
            if let Some(PubSubOut::FloodsubMessage{ data, ..}) = r1.recv().await {
              println!("get message {:?}", String::from_utf8(data).unwrap());
            }
          }
        });
        

        ipfs.clone().subscribe(TopicBuilder::new("P2VtoJ7jk").build()).await;

        let topic = TopicBuilder::new("hello").build();
        ipfs.clone().subscribe(topic.clone()).await;
        
        // wait for peer connect
        let _task = Delay::new(Instant::now() + Duration::from_millis(1000)).compat().await.unwrap();
        ipfs.publish(TopicHash::from(topic.clone()), Vec::from("hallo this is my first message\n")).await;


        loop {
          let _task = Delay::new(Instant::now() + Duration::from_millis(1000)).compat().await.unwrap();
        }
      }
      .unit_error()
      .boxed()
      .compat(),
    )
    .unwrap();
}
