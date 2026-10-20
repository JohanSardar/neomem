use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::{LogOptions, send_logs_to_tracing};
use neomem_core::{ModelError, ModelId};

const EVERY_LAYER: u32 = 999;

type Job = Box<dyn for<'model> FnOnce(&'model LlamaModel, &mut LlamaContext<'model>) + Send>;

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct LocalModel {
    id: ModelId,
    jobs: Sender<Job>,
}

impl LocalModel {
    pub(crate) fn load<Facts: Send + 'static>(
        path: &Path,
        prepare: impl FnOnce(&LlamaModel) -> Result<(LlamaContextParams, Facts), ModelError> + Send + 'static,
    ) -> Result<(Self, Facts), ModelError> {
        if !path.is_file() {
            return Err(ModelError::Unavailable(format!("the model file {} is missing", path.display())));
        }
        let (jobs, waiting) = mpsc::channel::<Job>();
        let (ready, loaded) = mpsc::sync_channel(1);
        let owned_path = path.to_path_buf();
        thread::Builder::new()
            .name(format!("llama.cpp {}", file_stem(path)))
            .spawn(move || serve(&owned_path, prepare, &ready, &waiting))
            .map_err(|error| ModelError::Unavailable(format!("starting the llama.cpp thread: {error}")))?;
        let facts = loaded.recv().map_err(|_| stopped())??;
        Ok((Self { id: local_model_id(path), jobs }, facts))
    }

    pub(crate) fn id(&self) -> ModelId {
        self.id.clone()
    }

    pub(crate) fn run<Answer: Send + 'static>(
        &self,
        work: impl for<'model> FnOnce(&'model LlamaModel, &mut LlamaContext<'model>) -> Result<Answer, ModelError>
        + Send
        + 'static,
    ) -> Result<Answer, ModelError> {
        let (reply, answer) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |model, context| {
            let _ = reply.send(work(model, context));
        });
        self.jobs.send(job).map_err(|_| stopped())?;
        answer.recv().map_err(|_| stopped())?
    }
}

pub(crate) fn local_model_id(path: &Path) -> ModelId {
    ModelId::new(format!("llama.cpp:{}", file_stem(path)))
}

pub(crate) fn context_threads() -> i32 {
    let cores = thread::available_parallelism().map_or(4, usize::from);
    i32::try_from((cores / 2).clamp(1, 8)).unwrap_or(4)
}

pub(crate) fn failure(stage: &str, error: impl Display) -> ModelError {
    ModelError::Unavailable(format!("llama.cpp {stage}: {error}"))
}

fn serve<Facts>(
    path: &PathBuf,
    prepare: impl FnOnce(&LlamaModel) -> Result<(LlamaContextParams, Facts), ModelError>,
    ready: &SyncSender<Result<Facts, ModelError>>,
    waiting: &Receiver<Job>,
) {
    let backend = match backend() {
        Ok(backend) => backend,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let layers = if backend.supports_gpu_offload() { EVERY_LAYER } else { 0 };
    let model_params = LlamaModelParams::default().with_n_gpu_layers(layers);
    let model = match LlamaModel::load_from_file(backend, path, &model_params) {
        Ok(model) => model,
        Err(error) => {
            let _ = ready.send(Err(failure("could not load the model", error)));
            return;
        }
    };
    let opened = prepare(&model).and_then(|(context_params, facts)| {
        let context = model
            .new_context(backend, context_params)
            .map_err(|error| failure("could not create a context", error))?;
        Ok((context, facts))
    });
    let mut context = match opened {
        Ok((context, facts)) => {
            let _ = ready.send(Ok(facts));
            context
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    tracing::info!(model = %file_stem(path), gpu = backend.supports_gpu_offload(), "llama.cpp model loaded");
    for job in waiting {
        job(&model, &mut context);
    }
}

fn backend() -> Result<&'static LlamaBackend, ModelError> {
    BACKEND
        .get_or_init(|| {
            send_logs_to_tracing(LogOptions::default());
            LlamaBackend::init().map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| failure("could not start", error))
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map_or_else(|| path.display().to_string(), |stem| stem.to_string_lossy().into_owned())
}

fn stopped() -> ModelError {
    ModelError::Unavailable("the llama.cpp thread stopped".to_owned())
}
