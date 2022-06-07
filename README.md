# RustSBI 在 VisionFive V1 主板的支持软件

这个项目的目的是在StarFive Visionfive V1主板上支持RustSBI。
RustSBI是一个引导程序环境；主板上电时，RustSBI将会先行启动，而后，它将会找到一个可引导的操作系统，引导启动这个操作系统。
在启动后，RustSBI仍然常驻后台，提供操作系统需要的功能。
RustSBI的设计完全符合RISC-V SBI规范标准，只要支持此标准的操作系统，都可以使用RustSBI引导启动。

感谢洛佳的RustSBI Hifive Unmatched项目：[rustsbi/rustsbi-hifive-unmatched: RustSBI support on SiFive FU740 board; FU740 is a five-core heterogeneous processor with four SiFive U74 cores, and one SiFive S7 core (github.com)](https://github.com/rustsbi/rustsbi-hifive-unmatched)，本项目参考此项目

## 编译和运行

这个项目使用xtask框架，可以使用以下指令来编译：

```shell
cargo image
```

（如果增加--release参数，说明编译的是不带调试符号的release版本）

刷入我修改的DDRinit程序，按照提示操作，将rustsbi-jh7100.image刷到内存指定区域即可

[Luchangcheng2333/JH7100_ddrinit (github.com)](https://github.com/Luchangcheng2333/JH7100_ddrinit)

## Rust版本

编译这个项目至少需要`rustc 1.59.0-nightly (c5ecc1570 2021-12-15)`的Rust版本。

## 有用的链接

- [Homepage | RVspace](https://rvspace.org/)
