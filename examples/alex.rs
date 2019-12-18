use futures::join;
use futures::{FutureExt, TryFutureExt};
use ipfs::{Ipfs, IpfsOptions, Ipld, TestTypes, IpfsPath};

fn main() {
  let options = IpfsOptions::<TestTypes>::default();
  env_logger::Builder::new().parse_filters(&options.ipfs_log).init();

  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on_all(
      async move {
        let (ipfs, fut) = Ipfs::new(options).start().await.unwrap();
        tokio::spawn(fut.unit_error().boxed().compat());

        let block1: Ipld = "block1".to_string().into();
        let block2: Ipld = "block2".to_string().into();
        let f1 = ipfs.put_dag(block1);
        let f2 = ipfs.put_dag(block2);
        let (res1, res2) = join!(f1, f2);

        let root: Ipld = vec![res1.unwrap(), res2.unwrap()].into();
        ipfs.put_dag(root).await.unwrap();
        let path = IpfsPath::from_str("/ipfs/zdpuB1caPcm4QNXeegatVfLQ839Lmprd5zosXGwRUBJHwj66X").unwrap();
        let _f2 = ipfs.get_dag(path.sub_path("1").unwrap());
      }
      .unit_error()
      .boxed()
      .compat(),
    )
    .unwrap();
}
