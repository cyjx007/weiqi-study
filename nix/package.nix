{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "weiqi-study";
  version = "0.2.1";

  # flake 会把源码过滤成 git 跟踪的文件，target/ 和 katago 软链不会进来
  src = lib.cleanSource ../.;

  cargoLock.lockFile = ../Cargo.lock;

  # 单测不需要 GPU、不需要网络
  doCheck = true;

  # 默认的 cargoInstallHook 会把 target/<triple>/release 下的可执行文件装进 $out/bin，
  # 两个 bin（go-review、weiqi-tui）都在里面，不用自己写 installPhase。

  meta = {
    description = "Local Go study tools: KataGo review + terminal board";
    license = lib.licenses.mit;
    mainProgram = "weiqi-tui";
  };
}
