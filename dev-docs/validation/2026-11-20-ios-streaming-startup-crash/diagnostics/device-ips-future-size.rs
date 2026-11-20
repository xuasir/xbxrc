use std::{future::Future, mem::size_of};
use xbox_ios_bridge::XboxStreamSession;
fn future_size<F: Future>(_: impl FnOnce(&'static XboxStreamSession) -> F) -> usize { size_of::<F>() }
fn main() {
    println!("start={}", future_size(|s| s.start()));
    println!("exchange_offer={}", future_size(|s| s.exchange_offer(1, String::new())));
    println!("close={}", future_size(|s| s.close()));
    println!("boxed_start={}", future_size(|s| Box::pin(s.start())));
}
