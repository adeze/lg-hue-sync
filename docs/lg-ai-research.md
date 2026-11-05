# LG AI, capture and tone mapping

Status (2026-11-05): ZipDepth and depth-AI lighting experiments are shelved at the
user’s request. No model or depth-AI weighting was enabled in live lighting.
Retain offline evidence; do not resume integration without new direction.

Observed locally on 2026-11-05. Native tracing establishes available code paths, not their current execution. Proprietary binaries and model weights remain in the private home-theatre archive; none are bundled with QSync.

## Genre diagnostics

The cached GENRE getter returns four scores and a selected data key. The verified consumer maps them to Standard, Sports, Animation and Movie. These are merged native genre scores, not the six raw GNET network probabilities. SQM and SNET's four outputs must not inherit these labels.

QSync displays this cache without enabling LG AI or subscribing to native callbacks. A fresh cache read does not establish fresh inference. The panel is diagnostic only; it does not adjust lighting.

## Sharing capture

One suitable shared source could replace QSync's independent capture acquisition, while QSync would still need frame conversion, sampling and light output. However, LG's AI inputs include small crops and derived spatial/temporal feature planes. Those are not established substitutes for complete colour frames needed by screen-edge sampling. Access, colour/range/transfer-function fidelity, cadence, buffer ownership and coexistence must be proven before replacing capture.

[PicCap's known issues](https://github.com/TBSniller/piccap#known-issues) document capture dropouts when several LG AI options compete with its capture backend. This is evidence of coexistence problems, not proof that LG's inference pipeline fails on its own or that a shared frame API is available.

## Spatial information and depth

Spatial tone mapping uses image positions and neighbourhoods. Sky above ground is an example of spatial layout; identifying sky is a separate semantic task. Image depth estimates describe relative near/far structure and are another capability. QSync's existing Hue depth influence describes physical room placement, not image depth. Relative scene depth cannot directly be equated to lamp coordinates or scene distance in metres.

A depth model could supply optional sampling weights, but its value must be demonstrated against simpler sampling. Evaluate temporal stability, camera cuts, dark scenes, subtitles and graphics. Depth is not automatically a better lighting target.

## LUTs and TV tone mapping

[ColourSpace's LG integration](https://lightillusion.com/lg_manual.html) explicitly supports the C1, 1D/3D LUT upload and separate HDR peak/roll-off controls. A 1D LUT supplies channel curves; a 3D LUT maps RGB to RGB, not screen XYZ or depth. A static calibration LUT can encode tonal transforms but is not the whole scene-adaptive or spatial tone-mapping algorithm. [LUT definitions](https://lightillusion.com/what_are_luts.html).

[Calman's custom tone-mapping guide](https://www.portrait.com/resource-center/lg-2019-custom-tone-mapping/) describes panel peak luminance, mastering peaks and roll-off points. This does not establish an arbitrary per-frame local tone-map upload interface.

Readback is not guaranteed. ColourSpace notes that LUT download is unavailable on some TVs; the [bscpylgtv author's documentation](https://github.com/chros73/bscpylgtv#calibration-functionality-in-full-version) warns that getters can fail or return postprocessed LUTs incorporating other settings. An extracted table is not automatically a factory backup. Improving calibration requires measured panel response and validation. No calibration upload, service-menu change or TV picture adjustment was performed.

## Custom inference feasibility

Local evidence establishes GPU compute used for LG preprocessing, TFLite-related native interfaces and a separate proprietary LNE firmware/weight path for SNET. It does not establish an accessible generic GPU delegate, custom LNE compiler, operator support or performance headroom. The TV's native runtime also requires ARM32 soft-float and glibc-compatible packaging.

Read-only observations on the configured TV establish `chip=O20B0` from `/proc/cmdline`, four cores with Arm implementer `0x41` and CPU part `0xd05`, and `Mali-G51 3 cores r1p1 0x7090` from `/sys/class/misc/mali0/device/gpuinfo` (also returned through `/sys/devices/platform/mali.0/gpuinfo`). [Arm's CPU identification example](https://developer.arm.com/community/arm-community-blogs/b/tools-software-ides-blog/posts/using-arm-models-for-software-development-and-performance-analysis-for-the-cortex-a75-and-cortex-a55) maps `0xd05` to Cortex-A55. These observations supersede generic model tables claiming A73/G52 hardware for this device. `/proc/meminfo` reported 2678680 KiB total and 725412 KiB available, approximately 2.55 GiB and 692 MiB respectively; available memory is only a transient snapshot, not an inference allocation budget.

There is a concrete historical LNE compiler lead: an [LG/AWS article](https://aws.amazon.com/blogs/apn/how-to-deploy-ai-inference-on-the-edge-with-the-lg-aiot-board-and-aws-iot-greengrass/) and [LG converter guide](https://github.com/lge-aws-dist/aws_gg_lambda/blob/master/doc/LG_AIoT_Board_LNE_Converter_Optimizer.pdf) document Caffe/TFLite conversion to `.lne`, quantization, `lne_tflite.Interpreter` and a native C++ interface. This concerns LG8111 AIoT hardware. Neither its performance figures nor its SDK compatibility are established for O20. Locating compatible SDK/runtime lineage is worthwhile; this is not proof that the TV accelerator already accepts arbitrary models.

[Depth Anything V2 Small](https://github.com/DepthAnything/Depth-Anything-V2) has 24.8 million parameters and default inference size 518; its FP32 parameters alone occupy about 99 MB before activations and runtime. Its Small weights use Apache-2.0. [Depth Anything 3 Small](https://github.com/ByteDance-Seed/Depth-Anything-3) is approximately 80 million parameters and is not a lighter default choice. Repository and checkpoint licenses must be checked for the selected teacher and student.

Fine-tuning alone does not reduce inference cost. Distillation into a compact student, reduced input resolution and quantization are distinct options requiring evaluation. Do not infer transformer throughput from LG's much smaller native classifiers. [Arm NN's current documentation](https://arm-software.github.io/armnn/latest/) identifies it as legacy; existing Mali/GLES symbols are not grounds for adding that runtime.

### Hugging Face shortlist

The Hugging Face connector was unavailable; public Hub pages and upstream sources were inspected instead. No weights were downloaded or executed.

| Candidate | Verified artifact or architecture | Relevance and limit |
| --- | --- | --- |
| [ZipDepth](https://github.com/fabiotosi92/ZipDepth), [Hub mirror](https://huggingface.co/pablovela5620/zipdepth) | Approximately 6.1M parameters; already distilled from Depth Anything V2 Large. Upstream provides an unfold-free mobile/CPU/NPU variant and ONNX export. | Preferred modern lightweight quality candidate. Default inference preserves aspect ratio with shorter side 384; fixed-shape exports must preserve useful TV regions. No O20 benchmark. |
| [ZipDepth ExecuTorch](https://huggingface.co/software-mansion/ZipDepth-ExecuTorch) | Published XNNPACK FP32 artifact approximately 24.6 MB; separate Vulkan/Core ML artifacts. | Existing CPU export is useful evidence of deployability. Requires compatible ExecuTorch runtime; Vulkan and Core ML artifacts are not established TV routes. |
| [MiDaS small LiteRT conversion](https://huggingface.co/litert-community/MiDaS-small) | CNN, EfficientNet-Lite3 backbone; 256x256 RGB input; FP16 TFLite approximately 33 MB. | Useful conversion/runtime baseline with documented builtin operators. Published phone and Raspberry Pi results do not predict TV performance. |
| [ST FastDepth model card](https://huggingface.co/STMicroelectronics/fastdepth) | MobileNetV1 encoder/decoder; INT8 TFLite described by vendor. | Smaller edge-oriented baseline. The inspected Hub root contains documentation only; weight provenance and actual artifact retrieval require follow-up. |

[Arm's ZipDepth package](https://huggingface.co/Arm/zipdepth-fp16-executorch) provides another deployment reference, but documents square-crop limitations, no temporal consistency and an early-access engine path. It is not a ready-made TV integration. Likewise, Snapdragon/NPU-optimized Depth Anything exports do not establish LG accelerator compatibility. Parameter counts and model file sizes are not working-memory or latency measurements.

Recommendation: evaluate an unchanged edge-friendly ZipDepth candidate and a simpler baseline before planning new distillation. Keep LG's cached genre diagnostic separate; a depth map does not replace genre classification. No purpose-built QSync lighting model was established by this search.

Before selecting a runtime or deploying a model:

1. Compare representative offline clips with current sampling, cheap image features, teacher depth and a compact student. Hold out programmes/scenes from training.
2. Verify conversion, operator coverage and quantized output parity locally.
3. With explicit direction for execution on the TV, benchmark a bounded latest-frame worker against QSync alone. Record inference p50/p95, memory, CPU, frame age, capture drops and sender timing. Test CPU first; qualify any GPU/accelerator path independently.
4. Keep inference outside the capture/output deadline. Choose cadence and resolution from measured benefit and available headroom, not guessed FPS or advertised AI capability.

No custom model, runtime dependency or TV benchmark is included in the diagnostics change.
