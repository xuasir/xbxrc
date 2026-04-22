use ohmygamepad_core::{
    spawn_input_runtime, InputCoreConfig, InputRuntimeHandle, StreamSink, UiSink,
};

use crate::{Sdl3Backend, Sdl3BackendConfig, Sdl3Source, Sdl3SourceInitError};

pub fn spawn_sdl3_input_runtime<TUiSink, TStreamSink>(
    core_config: InputCoreConfig,
    backend_config: Sdl3BackendConfig,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
) -> Result<InputRuntimeHandle, Sdl3SourceInitError>
where
    TUiSink: UiSink + Send + 'static,
    TStreamSink: StreamSink + Send + 'static,
{
    let backend = Sdl3Backend::new(backend_config)?;
    Ok(spawn_input_runtime(
        core_config,
        backend,
        ui_sink,
        stream_sink,
    ))
}

pub fn spawn_sdl3_input_runtime_with_source<TSource, TUiSink, TStreamSink>(
    core_config: InputCoreConfig,
    backend_config: Sdl3BackendConfig,
    source: TSource,
    ui_sink: TUiSink,
    stream_sink: TStreamSink,
) -> InputRuntimeHandle
where
    TSource: Sdl3Source + Send + 'static,
    TUiSink: UiSink + Send + 'static,
    TStreamSink: StreamSink + Send + 'static,
{
    let backend = Sdl3Backend::with_source(backend_config, source);
    spawn_input_runtime(core_config, backend, ui_sink, stream_sink)
}
