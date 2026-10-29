# Comparación medida de modelos E2

Misma petición: función Python suma. Contexto solicitado y observado: 4096. Generación máxima: 512; temperatura: 0.2; thinking desactivado. Una muestra por modelo.

| Modelo | Respuesta total | Carga | Primer texto | VRAM del modelo según Ollama |
|---|---:|---:|---:|---:|
| `huihui_ai/qwen3.5-abliterated:2B` | 86.25 s | 61.38 s | 86.06 s | 1.49 GiB |
| `huihui_ai/qwen3-abliterated:4b-v2` | 33.81 s | 33.62 s | 33.70 s | 2.96 GiB |
| `huihui_ai/qwen3.5-abliterated:4B` | 42.05 s | 41.70 s | 41.86 s | 2.92 GiB |

Los tres devolvieron texto de una función suma mediante varios fragmentos y terminaron con done_reason=stop. El código no se ejecutó. Los tiempos de carga y caché difieren: no permiten una clasificación general ni elegir automáticamente un modelo principal.

El producto mantiene `ollama.model=null`. Selección explícita por lista, configuración o `--model`. Herramientas, agentes y liberación siguen pendientes.

Datos completos, digests y fuentes: [comparación JSON](2026-10-20-e2-comparacion.json). La memoria del modelo se obtiene de `/api/ps`; GPU global y consumo por proceso están separados en las evidencias `e2-live-*.json`.
