# AnyFile

一个面向 Windows 平台的轻量、冷启动极快的开源多用途文件查看器，提供内容优先的沉浸式界面与对标 iOS 的物理弹簧动效。

## Language

**File Entry（文件项）**:
磁盘上当前打开或位于同目录序列中的单个文件对象，包含其路径与探测出的媒体大类。
_Avoid_: Item, Asset, Resource

**Media Category（媒体大类）**:
文件项的顶层分类——图像（Image）、视频（Video）、音频（Audio）或文本（Text）——决定启用哪个查看器视图以及同目录切换的默认筛选范围。
_Avoid_: File Type, Format Group, Kind

**Viewer（查看器视图）**:
负责呈现特定媒体大类并提供对应交互能力的专用视图（图像视图、视频视图、音频视图、文本视图）。
_Avoid_: Previewer, Panel, Renderer

**Sibling Sequence（同目录序列）**:
与当前文件项位于同一目录下的可浏览文件有序列表，按自然排序排列，默认仅包含当前媒体大类（可切换为全部支持格式）。
_Avoid_: Playlist, Directory List, Neighbors

**Viewing Mode（查看态）**:
文本视图的默认只读状态，提供语法高亮、行号与查找能力，不修改磁盘文件。
_Avoid_: Preview Mode, Read Mode

**Editing Mode（编辑态）**:
文本视图的可编辑状态，支持纯文本修改、撤销/重做、查找替换，以及保留原编码与换行符的保存（默认显式保存，可选自动保存）。
_Avoid_: Write Mode, Code Editor Mode

**Motion Profile（动效配置）**:
驱动界面过渡与直接操作反馈的物理弹簧参数集合；当系统开启“减弱动态效果”时自动降级为瞬时切换。
_Avoid_: Animation Style, Transition Theme
