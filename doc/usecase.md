### case

"how does camera stream sent out?"

--

"User is working on  building a FSD perception node, which is inside  a FSD system structure, camera stream is sent out from a Unity Simulation for dev. On real product is will be by a usb camera hardware.   
Relating camera streaming code is in Project Sailsim, relating files:
1. /path/to/camera.cs
2. /pat/to/rtps_steam.cs
As a system design and architect, please Explan:
How does camera stream send out from simulator and stream to rtsp Server."


### case

“我这个无人车系统的 camera 推 RTSP 流到 Unity 里总是卡顿，是为什么？”

--

你是一个熟悉 Unity、无人驾驶仿真、RTSP 流媒体的资深工程师。

当前背景：
	•	我在做一个无人驾驶系统仿真。
	•	作者用 Unity 模拟车和传感器，用 Unity 的游戏视角模拟摄像头。
	•	通过 RTSP 推流到地址 rtsp://127.0.0.1:8554/front。

现在的问题：
	•	我在 Unity 中接收这个 RTSP 视频流时，经常出现明显卡顿和延迟。
	•	希望分析可能原因，并给出排查步骤。

请按照以下结构回答：
	1.	可能原因列表（网络、编码、Unity 设置、RTSP 服务器等）
	2.	每个原因对应的排查步骤
	3.	如果要减小延迟，推荐的配置（编码参数、分辨率、帧率等）
