//! Checks the streaming provider against a mock server:
//! cancel mid-stream, idle timeout, keep-alive queue, non-streaming answer.
//!
//!     cargo run -p corydoras-core --example provider_check -- http://127.0.0.1:PORT

use corydoras_core::activity::Activity;
use corydoras_core::provider::{self, Part, Provider, Thinking};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let base = std::env::args().nth(1).expect("base URL");
    let parts = [Part::Text("hi".into())];

    // 1. Cancel mid-stream: the call must stop within a moment of cancel().
    let act = Arc::new(Activity::default());
    act.begin(1, "synthesize", "l", "L", None, None, "Writing");
    act.call_begin(1, 1, "Waiting for the model", 1000);
    let p = Provider::new(&format!("{base}/slow"), "m", "k")?;
    let a2 = act.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        a2.cancel(Some(1));
    });
    let t = Instant::now();
    let r = p.chat("s", &parts, false, 1000, Thinking::Off, Some(&act)).await;
    let e = r.err().expect("cancel must end the call with an error");
    let c = act.current().unwrap();
    println!(
        "cancel: stopped after {:.1} s, is_cancelled={}, received ~{} chars first, stream state '{}'",
        t.elapsed().as_secs_f64(),
        provider::is_cancelled(&e),
        c.output_chars,
        c.stream
    );
    assert!(provider::is_cancelled(&e));
    assert!(t.elapsed() < Duration::from_secs(4));
    assert!(c.output_chars >= 2);

    // 2. Idle timeout: no bytes for the idle limit -> transient "Stalled" error.
    let act = Activity::default();
    act.begin(2, "analyze_item", "l", "L", None, None, "Analyzing");
    let p = Provider::new(&format!("{base}/silent"), "m", "k")?.with_idle(Duration::from_secs(2));
    let t = Instant::now();
    let e = p.chat("s", &parts, false, 100, Thinking::Off, Some(&act)).await.err().expect("idle must fail");
    println!("idle: {:.1} s, transient={}, message: {e}", t.elapsed().as_secs_f64(), provider::is_transient(&e));
    assert!(provider::is_transient(&e));
    assert!(t.elapsed() < Duration::from_secs(4));

    // 3. Keep-alive comments while queued, then the answer.
    let act = Arc::new(Activity::default());
    act.begin(3, "analyze_item", "l", "L", None, None, "Analyzing");
    act.call_begin(1, 1, "Waiting for the model", 100);
    let p = Provider::new(&format!("{base}/queued"), "m", "k")?;
    let a3 = act.clone();
    let probe = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        a3.current().unwrap().stream
    });
    let r = p.chat("s", &parts, false, 100, Thinking::Off, Some(&act)).await?;
    let mid = probe.await?;
    println!("queued: state during wait '{mid}', answer '{}', tokens {}/{}", r.text, r.input_tokens, r.output_tokens);
    assert_eq!(mid, "queued");
    assert_eq!(r.text, "OK");

    // 4. A server that does not stream.
    let p = Provider::new(&format!("{base}/plain"), "m", "k")?;
    let r = p.chat("s", &parts, false, 100, Thinking::Off, None).await?;
    println!("plain: answer '{}', finish {}, usage estimated {}", r.text, r.finish_reason, r.usage_estimated);
    assert_eq!(r.text, "OK");
    assert!(!r.usage_estimated);

    println!("ALL PROVIDER CHECKS PASSED");
    Ok(())
}
