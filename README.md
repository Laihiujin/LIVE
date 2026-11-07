# LIVE

<img src="assets/branding/LIVE.png" width="128" alt="LIVE" />

[![Build](https://img.shields.io/github/actions/workflow/status/Laihiujin/LIVE/build-dmg.yml?branch=main&label=Build)](../../actions)
[![Release](https://img.shields.io/github/v/release/Laihiujin/LIVE)](../../releases)
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)

[LIVE](https://github.com/Laihiujin/LIVE) 是一个可视化的矩阵直播录制与切片工具，开箱即录。支持 B 站、抖音、虎牙、快手、TikTok、微信视频号 6 个平台，可内置浏览器扫码登录，也可用访客模式免登录录制，并能重载以往缓存的直播记录。

## 支持平台

| 平台 | 添加方式 | 流格式 | 弹幕 | 登录 | 访客模式 |
|---|---|---|---|---|---|
| **B 站** | 直播间链接 / 房间号，可批量 | HLS (fmp4/ts) | ✅ WebSocket | 扫码 / 手动 Cookie | ✅ |
| **抖音** | 直播间链接 / 房间号，可批量 | HLS | ✅ WebSocket | 扫码 / 手动 Cookie | ✅ |
| **虎牙** | 直播间链接 / 房间号，可批量 | HLS | — | 扫码 / 手动 Cookie | ✅ |
| **快手** | 直播间链接 / 房间号，可批量 | HLS / FLV / RTMP | ✅ WebSocket | 扫码 / 手动 Cookie | ✅ |
| **TikTok（海外）** | `@用户名` / 数字房间 ID，可批量 | HLS / FLV | ✅ HTTP 轮询 | 内嵌 WebView / 手动 Cookie | ✅ |
| **微信视频号** | 自动抓取 / 粘贴媒体地址 | HLS / FLV / MP4 | — | 不需要 | — |

**访客模式**：无需登录即可录制。各平台对未登录请求有反爬校验，本项目通过逆向工程绕过校验，并对访客 Cookie 实现自动刷新。

**登录模式**：应用内嵌 WebView 弹窗，直接扫码登录获取完整 Cookie，通常可解锁更高分辨率。

---

各平台的具体说明：

**B 站**：支持 4K 最高画质（需登录），同时拉取 AVC/HEVC 双编码，弹幕通过官方 WebSocket 协议实时接收。

**抖音**：拉取 HLS origin 流，弹幕通过官方 WebSocket 协议实时接收。

**虎牙**：通过解析直播页 JS 数据获取签名 HLS 流，暂未接入弹幕。

**快手**：优先 HLS，不可用时自动回退 FLV，弹幕通过 WebSocket 接收。

**TikTok（海外）**：需要代理访问，支持按用户名 `@xxx` 或数字房间 ID 添加；录制前可检测当前代理是否可用。

**微信视频号**：视频号直播间没有可复制的链接，流地址只出现在电脑版微信的网络请求里。
- **自动抓取（Windows / macOS）**：首次使用先安装本机生成的证书，点「开始抓取」后在电脑版微信进入直播间，列表里出现地址就点「使用这个」添加录制。抓取期间临时接管系统代理（最长 10 分钟），结束后自动恢复。证书只对腾讯直播相关域名生效，其他网站的连接原样转发。
- **粘贴媒体地址**：也可以直接粘贴 m3u8 / flv / mp4 地址，或 wechatVideoDownload、res-downloader 导出的资源信息。
- 不读取微信账号，没有弹幕，也不能解密加密视频。地址带签名，过期后需要重新抓取。细节见 [docs/wechat-local.md](docs/wechat-local.md)。

## 支持系统

| 系统 | 安装包 |
|---|---|
| Windows 10/11 x64 | 普通版；有 NVIDIA 显卡可选 CUDA 版，加速字幕识别 |
| macOS 13.3+（Apple Silicon / Intel） | 分别下载 arm64 / x64 的 DMG |
| Linux x64 | AppImage / deb / rpm |

---

## 相对原版的其他改动

**迁移缓存并重载识别**
- 从旧目录迁移缓存到当前缓存路径,并重建录播记录；
- 支持将其他文件夹的直播间录播记录导入重载m3u8/ts视频片段，避免卸载软件出现直播记录消失；

**账号管理增强**

- 所有平台均支持应用内嵌 WebView 扫码登录，无需手动复制 Cookie；
- 登录账号与访客账号双轨并行，可同时开启；
- 访客cookie 自动刷新；
- 逆向功能二维码api登录及内置浏览器登录；

### 其他实用功能
- **重载直播间**：重新加载直播间，减少删除再重新添加的步骤，适用于直播间访问频繁
- **反向代理检测**：TikTok 录制前可检测当前代理是否可用
- **批量直播间链接添加直播间**：（同平台可批量添加直播间，单次最多20个）
- **B 站投稿开关**：可在设置中关闭 B 站投稿功能，仅保留本地切片
- **其余功能**：录制、实时回放、弹幕导出、封面编辑、B 站投稿等

## 安装

前往 [Releases](../../releases) 下载对应平台的安装包。版本号从 v1.0.0 开始，文件统一命名为 `LIVE_<版本>_<系统>-<架构>`：

| 文件 | 说明 |
|---|---|
| `LIVE_x.y.z_windows-x64-setup.exe` | Windows |
| `LIVE_x.y.z_windows-x64-cuda-setup.exe` | Windows，NVIDIA 显卡加速字幕识别（CUDA） |
| `LIVE_x.y.z_macos-arm64.dmg` | macOS Apple Silicon（M 系列芯片） |
| `LIVE_x.y.z_macos-x64.dmg` | macOS Intel |
| `LIVE_x.y.z_linux-x64.AppImage` / `.deb` / `.rpm` | Linux |

## 致谢

本项目参考 [@Xinrea](https://github.com/Xinrea) 的 [bili-shadowreplay](https://github.com/Xinrea/bili-shadowreplay) 迭代，致敬原作者~
