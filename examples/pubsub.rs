use futures::{FutureExt, TryFutureExt};

use ipfs::{Ipfs, IpfsOptions, TestTypes, TopicBuilder, TopicHash, PubSubOut};
use async_std::{io, task};
use futures::{future, prelude::*};
use futures::task::{Poll, Context};

fn main() {
  let options = IpfsOptions::<TestTypes>::default();
  env_logger::Builder::new().parse_filters(&options.ipfs_log).init();

  task::block_on(async move {
    let (ipfs, fut) = Ipfs::new(options).await.start().await.unwrap();
    task::spawn(fut);

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let topic = TopicBuilder::new("hello").build();

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
    ipfs.publish(topic.clone().into(), Vec::from("hallo this is my first message\n")).await;

    task::block_on(future::poll_fn(move |cx: &mut Context| {
      loop {
        match stdin.poll_next_unpin(cx) {
            Poll::Ready(Some(line)) => {
              let line: String = line.unwrap();
              // one poll is enough for demo
              task::spawn(ipfs_cp.clone().publish_any(topic.clone().into(), Vec::from(line)));
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
