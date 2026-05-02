fn main() {
	if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
		println!("cargo:rerun-if-changed=build/yammm.manifest");
		let _ = embed_resource::compile(
			"build/yammm.manifest",
			embed_resource::NONE,
		);
	}
}
