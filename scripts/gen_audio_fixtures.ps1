# 生成音频视图测试用的 1 秒样例（MP3/WAV/FLAC/OGG/M4A/AAC），其中 MP3/FLAC/M4A 内嵌 32x32 封面与标签。
# 依赖本机 ffmpeg；生成结果已提交到 tests/fixtures/audio，日常测试不需要重新运行。
$ErrorActionPreference = "Stop"
$out = Join-Path $PSScriptRoot "..\tests\fixtures\audio"
New-Item -ItemType Directory -Force $out | Out-Null
$cover = Join-Path $out "cover.png"
ffmpeg -y -v error -f lavfi -i "color=c=0x2563EB:s=32x32" -frames:v 1 $cover
$src = @("-f","lavfi","-i","sine=frequency=440:duration=1:sample_rate=44100")
$meta = @("-metadata","title=TestTrack","-metadata","artist=AnyFile","-metadata","album=SampleAlbum")
ffmpeg -y -v error @src -ac 2 (Join-Path $out "sample.wav")
ffmpeg -y -v error @src -i $cover -map 0:a -map 1:v -c:a libmp3lame -c:v copy -id3v2_version 3 @meta -disposition:v attached_pic (Join-Path $out "sample.mp3")
ffmpeg -y -v error @src -i $cover -map 0:a -map 1:v -c:a flac -c:v copy @meta -disposition:v attached_pic (Join-Path $out "sample.flac")
ffmpeg -y -v error @src -c:a libvorbis @meta (Join-Path $out "sample.ogg")
ffmpeg -y -v error @src -i $cover -map 0:a -map 1:v -c:a aac -c:v copy @meta -disposition:v attached_pic (Join-Path $out "sample.m4a")
ffmpeg -y -v error @src -c:a aac -f adts (Join-Path $out "sample.aac")
Remove-Item $cover
Get-ChildItem $out | Select-Object Name, Length
