# NVIDIA 驱动自动升级导致 CUDA 不可用：原理、恢复与预防

> 适用于 Ubuntu/Debian + NVIDIA 闭源驱动的 Linux 服务器（我们部署 voice-cli CUDA 档 / document-parser GPU 解析的机器都可能遇到）。本文不含具体机器信息，是通用的运维知识文档。

## 1. 现象

```console
$ nvidia-smi
Failed to initialize NVML: Driver/library version mismatch
```

同时会出现一组"矛盾"的表现：

- **一直在跑的 GPU 服务完全正常**（转写/解析一直在用 GPU，健康检查全绿）
- **任何新起的进程用不了 CUDA**（`nvidia-smi` 报错、新部署的服务报 CUDA 初始化失败、GPU 加速档探针失败）
- 机器本身不宕机、不掉卡，`lspci` 一切正常

这个状态**不会自行恢复**（直到重启，见 §4），也**不会恶化**——是稳定的"半新半旧"状态。

## 2. 原理

### 2.1 NVIDIA 驱动是"两半"的，且严格版本配对

| 一半 | 位置 | 生命周期 |
|------|------|---------|
| 内核模块（`nvidia.ko` / `nvidia_uvm.ko`） | 加载进内核 | **开机加载，驻留到关机**，磁盘文件换了也不影响已加载的模块 |
| 用户态库（`libnvidia-ml.so` / `libcuda.so` 等） | 磁盘上的文件 | 每个**新进程**启动时从磁盘加载 |

NVIDIA 强制这两半**逐版本精确匹配**：用户态库初始化时校验内核模块版本，差一个小版本号就直接拒绝工作（`NVML mismatch` 即此报错）。这是 NVIDIA 上游的工程决策（闭源二进制、简化支持矩阵），发行版改不了。

对比：AMD 的方案（内核 amdgpu + Mesa 用户态）是向前兼容设计，自动升级 Mesa 不会出现这种"拒绝初始化"——所以这个问题是 **NVIDIA 特有**，不是 Linux 通病。

### 2.2 unattended-upgrades 制造了"半新半旧"

Ubuntu 24.04 默认开启 `unattended-upgrades`（自动安全更新），且 NVIDIA 驱动的发行版包（`linux-modules-nvidia-*` / `nvidia-utils-*` / `libnvidia-*`）也在更新范围内。NVIDIA 驱动有真实的漏洞历史（提权、显存越权），从安全角度它**倾向于照常更新**。

于是当 NVIDIA 出安全点更新（例如 `595.71.05 → 595.84`）：

1. 自动升级把**磁盘上的用户态库全部换新**
2. **已加载的内核模块还是旧版**（换不了，模块驻留到关机）
3. 新进程：新库 ↔ 旧内核模块 → 版本失配 → 拒绝初始化

发行版的配套假设是"内核类更新装完后等你重启生效"（与内核自身更新同一模式，也提供了 `Automatic-Reboot` 选项但默认关闭）。对会正常重启的桌面/常规服务器，这个窗口期几乎无感；**对 7×24 不重启的 GPU 生产机**，这个状态会一直持续。

### 2.3 为什么老进程没事

Linux 替换磁盘文件（rename 原子替换）不影响**已打开的内存映射**：升级前就在跑的进程，内存里映射的还是旧版库文件，旧库配旧内核模块——完全匹配，继续正常用 GPU。这就是"服务还活着但起新的就死"的原因，也引出下面的关键纪律。

## 3. 影响判定与关键纪律

| 对象 | 影响 |
|------|------|
| 在跑的 GPU 服务 | **不受影响**，可无限期维持 |
| 新起的进程（部署新服务、重启服务进程） | **起不了 CUDA** |

> ⚠️ **关键纪律：此状态下绝不 `systemctl restart` 在跑的 GPU 服务**——重启会杀掉持旧上下文的进程，新进程起不了 CUDA，服务就回不来了。

## 4. 恢复方案（三选一）

### 方案一：什么都不做（合法）

如果机器的定位就是"老服务跑着、不部署新东西"，现状可以无限期维持。没有倒计时。

### 方案二：维护窗口重启（推荐，最干净）

重启后内核加载磁盘上的新模块 → 与磁盘上的新用户态天然匹配 → **一次到位、永久恢复**，且升到了带安全修复的新版本。

重启时**顺手做 §5 的 hold**，避免下次自动升级再断。

### 方案三：不重启恢复（用户态降级，精度要求高）

把磁盘上的用户态库**降回**内核驻留的旧版本，两半重新匹配，新进程立刻可用，全程不动任何运行中的进程：

```bash
# 1. 确认内核侧版本（这是要降回去的目标版本）
cat /proc/driver/nvidia/version

# 2. 查看当前磁盘上的新版本
dpkg -l | grep -E 'nvidia-utils|libnvidia-' 

# 3. 降级整串用户态包（必须降齐，降一半更糟）
sudo apt install \
  nvidia-utils-<系列>=<旧版本> \
  libnvidia-compute-<系列>=<旧版本> \
  libnvidia-gl-<系列>=<旧版本> \
  libnvidia-common-<系列>=<旧版本> \
  libnvidia-cfg1-<系列>=<旧版本> \
  # ...以 dpkg -l 实际列表为准
  # （旧版本号可用 apt list -a <包名> 查到）

# 4. 立即 hold 防止下次自动升级又断（见 §5）

# 5. 验证
nvidia-smi   # 应恢复正常
```

## 5. 预防：禁止 NVIDIA 驱动被自动升级

### 方案 A：`apt-mark hold`（推荐，最硬）

把 NVIDIA 相关包钉在当前版本——apt 的所有升级路径（手动 `apt upgrade`、unattended-upgrades）全部跳过：

```bash
# 一条命令 hold 所有已安装的 NVIDIA 包
sudo apt-mark hold $(dpkg -l | awk '/^ii/ && $2 ~ /nvidia/ {print $2}')

# 查看当前 hold 列表
apt-mark showhold
```

将来要升级驱动（维护窗口）：`apt-mark unhold` 同样的包 → `sudo apt full-upgrade` → **重启**（内核模块换新必须重启才生效）→ 验证 `nvidia-smi`。

### 方案 B：unattended-upgrades 黑名单（只挡自动升级）

编辑 `/etc/apt/apt.conf.d/50unattended-upgrades`，在 `Package-Blacklist` 块中加入前缀匹配：

```
Unattended-Upgrade::Package-Blacklist {
    "nvidia-";
    "libnvidia-";
    "linux-modules-nvidia-";
};
```

与 A 的区别：只拦自动升级，手动 `apt upgrade` 仍会升——适合"平时别动我驱动，维护窗口手动一把升"的用法。

### 方案 C：关闭整个 unattended-upgrades（不推荐）

```bash
# /etc/apt/apt.conf.d/20auto-upgrades
APT::Periodic::Unattended-Upgrade "0";
```

面太宽：会失去所有安全自动更新（SSH/OpenSSL 等也不再自动补）。为保显卡把整个攻击面敞开，得不偿失。GPU 驱动有自己的攻击面要权衡，但用 A/B 精准 hold 即可，不必走到这一步。

## 6. 诊断速查

```bash
# 两半各自的版本（不一致 = 中招）
cat /proc/driver/nvidia/version          # 内核模块版本
dpkg -l | grep nvidia-utils              # 磁盘用户态版本
nvidia-smi                               # 报 mismatch 即失配

# 自动升级是否开启
systemctl is-active unattended-upgrades
grep Unattended-Upgrade /etc/apt/apt.conf.d/20auto-upgrades

# 当前防护状态
apt-mark showhold                        # hold 列表
grep -A5 Package-Blacklist /etc/apt/apt.conf.d/50unattended-upgrades
```

## 7. 一句话总结

Ubuntu 的自动安全更新 + NVIDIA 的严格版本配对 + 生产机不重启，三者叠加制造出"老进程正常、新进程起不了 CUDA"的稳定病态。**不重启就维持现状（别动老进程）**；恢复靠重启或用户态降级；根治靠 `apt-mark hold` + 维护窗口手动升级驱动。
