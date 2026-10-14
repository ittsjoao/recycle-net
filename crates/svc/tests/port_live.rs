// Requires the netrbin driver loaded and SNetRBin stopped. Run with:
//   cargo test -p rnetrbin-svc --test port_live -- --ignored
use rnetrbin_svc::port::PortClient;

#[test]
#[ignore]
fn connect_and_configure_against_live_driver() {
    let c = PortClient::connect().expect("connect");
    c.configure(&["*.*".into()], &[], r"D:\RecycleBinNet").expect("configure");
    // Manual assertion: delete a file on a monitored volume and confirm a
    // .nrbnew + flag file appears under D:\RecycleBinNet.
}
