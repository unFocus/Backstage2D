fn main() {
    let stage_binary =
        std::env::current_exe().expect("cannot locate own executable").with_file_name("backstage_stage");
    relm4::RelmApp::new("dev.backstage2d.Tools").run::<backstage_tools::app::App>(stage_binary);
}
