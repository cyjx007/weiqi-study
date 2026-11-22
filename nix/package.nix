{
  lib,
  rustPlatform,
  pkg-config,
  gobject-introspection,
  wrapGAppsHook4,
  gtk4,
  libadwaita,
}:

rustPlatform.buildRustPackage {
  pname = "weiqi-study";
  version = "0.6.0";

  # flake 会把源码过滤成 git 跟踪的文件，target/ 和 katago 软链不会进来
  src = lib.cleanSource ../.;

  cargoLock.lockFile = ../Cargo.lock;

  # 单测不需要 GPU、不需要网络
  doCheck = true;

  nativeBuildInputs = [
    pkg-config
    gobject-introspection
    wrapGAppsHook4
  ];

  buildInputs = [
    gtk4
    libadwaita
  ];

  postInstall = ''
    install -Dm644 ${../packaging/weiqi-study.desktop} $out/share/applications/weiqi-study.desktop
  '';

  meta = {
    description = "Local Go study tools: KataGo review, terminal board and GTK4 window";
    license = lib.licenses.mit;
    mainProgram = "weiqi-tui";
  };
}
