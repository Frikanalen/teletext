// With the `decklink` feature, compile the C++ shim against the Desktop Video SDK.
fn main() {
    if std::env::var_os("CARGO_FEATURE_DECKLINK").is_none() {
        return;
    }
    // The SDK's Linux headers are vendored under vendor/decklink-sdk/include;
    // DECKLINK_SDK_INCLUDE overrides that.
    let inc = std::env::var("DECKLINK_SDK_INCLUDE")
        .unwrap_or_else(|_| "vendor/decklink-sdk/include".into());
    println!("cargo:rerun-if-env-changed=DECKLINK_SDK_INCLUDE");
    println!("cargo:rerun-if-changed=shim/decklink_shim.cpp");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(&inc)
        .file("shim/decklink_shim.cpp")
        .file(format!("{inc}/DeckLinkAPIDispatch.cpp"))
        .compile("decklink_shim");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rustc-link-lib=pthread");
}
