//! Server reflection: asks a running server for its own schema.
//!
//! The reflection service is itself gRPC, so its messages are declared here (the schema is small and stable)
//! rather than generated. `grpc.reflection.v1` is tried first and `v1alpha`, which older servers still
//! only offer, second. Both have the same messages.

use std::collections::HashSet;

use indexmap::IndexMap;
use prost::Message;
use prost_reflect::DescriptorPool;
use prost_types::FileDescriptorProto;
use tokio::sync::mpsc;
use tonic::{Code, Request, Status, Streaming, client::Grpc, transport::Channel};
use tonic_prost::ProstCodec;

#[derive(Clone, PartialEq, Message)]
struct ServerReflectionRequest {
    #[prost(string, tag = "1")]
    host: String,
    #[prost(oneof = "request::MessageRequest", tags = "3, 4, 7")]
    message_request: Option<request::MessageRequest>,
}

mod request {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageRequest {
        #[prost(string, tag = "3")]
        FileByFilename(String),
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        #[prost(string, tag = "7")]
        ListServices(String),
    }
}

#[derive(Clone, PartialEq, Message)]
struct ServerReflectionResponse {
    #[prost(string, tag = "1")]
    valid_host: String,
    #[prost(message, optional, tag = "2")]
    original_request: Option<ServerReflectionRequest>,
    #[prost(oneof = "response::MessageResponse", tags = "4, 6, 7")]
    message_response: Option<response::MessageResponse>,
}

mod response {
    use super::{ErrorResponse, FileDescriptorResponse, ListServiceResponse};

    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageResponse {
        #[prost(message, tag = "4")]
        FileDescriptorResponse(FileDescriptorResponse),
        #[prost(message, tag = "6")]
        ListServicesResponse(ListServiceResponse),
        #[prost(message, tag = "7")]
        ErrorResponse(ErrorResponse),
    }
}

#[derive(Clone, PartialEq, Message)]
struct FileDescriptorResponse {
    #[prost(bytes = "vec", repeated, tag = "1")]
    file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct ListServiceResponse {
    #[prost(message, repeated, tag = "1")]
    service: Vec<ServiceResponse>,
}

#[derive(Clone, PartialEq, Message)]
struct ServiceResponse {
    #[prost(string, tag = "1")]
    name: String,
}

#[derive(Clone, PartialEq, Message)]
struct ErrorResponse {
    #[prost(int32, tag = "1")]
    error_code: i32,
    #[prost(string, tag = "2")]
    error_message: String,
}

/// The value of a list request is ignored by the protocol, but some servers (Node ones) read an empty one as
/// no request at all, so it is never empty.
const LIST_ALL: &str = "*";
const V1: &str = "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo";
const V1_ALPHA: &str = "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo";

/// What a server reports about itself: the full schema and which of its services it exposes.
pub struct Reflected {
    pub pool: DescriptorPool,
    pub services: Vec<String>,
}

pub async fn reflect(channel: Channel) -> Result<Reflected, String> {
    match ask_server(channel.clone(), V1).await {
        Err(status) if status.code() == Code::Unimplemented => ask_server(channel, V1_ALPHA).await.map_err(describe),
        other => other.map_err(describe),
    }
}

fn describe(status: Status) -> String {
    match status.code() {
        Code::Unimplemented => "The server does not support reflection. Choose a .proto file instead.".to_owned(),
        code => format!("Reflection failed: {code:?}: {}", status.message()),
    }
}

struct Conversation {
    outbound: mpsc::Sender<ServerReflectionRequest>,
    inbound: Streaming<ServerReflectionResponse>,
}

impl Conversation {
    async fn ask(&mut self, message: request::MessageRequest) -> Result<response::MessageResponse, Status> {
        send(&self.outbound, message).await?;
        self.listen().await
    }

    async fn listen(&mut self) -> Result<response::MessageResponse, Status> {
        let reply = self
            .inbound
            .message()
            .await?
            .ok_or_else(|| Status::unavailable("The server ended the reflection stream."))?;
        reply.message_response.ok_or_else(|| Status::internal("The server sent an empty reflection reply."))
    }

    /// The files that answer a request, or `None` when the server says it has no such symbol or file.
    async fn files(&mut self, message: request::MessageRequest) -> Result<Option<Vec<FileDescriptorProto>>, Status> {
        match self.ask(message).await? {
            response::MessageResponse::FileDescriptorResponse(found) => found
                .file_descriptor_proto
                .iter()
                .map(|bytes| {
                    FileDescriptorProto::decode(bytes.as_slice())
                        .map_err(|e| Status::internal(format!("The server sent a malformed descriptor: {e}")))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            response::MessageResponse::ErrorResponse(_) => Ok(None),
            response::MessageResponse::ListServicesResponse(_) => {
                Err(Status::internal("The server answered a file request with a service list."))
            }
        }
    }
}

async fn send(
    outbound: &mpsc::Sender<ServerReflectionRequest>,
    message: request::MessageRequest,
) -> Result<(), Status> {
    let request = ServerReflectionRequest { host: String::new(), message_request: Some(message) };
    outbound.send(request).await.map_err(|_| Status::unavailable("The reflection stream closed."))
}

async fn ask_server(channel: Channel, path: &'static str) -> Result<Reflected, Status> {
    let (outbound, receiver) = mpsc::channel::<ServerReflectionRequest>(8);
    let stream =
        futures::stream::unfold(
            receiver,
            |mut receiver| async move { receiver.recv().await.map(|item| (item, receiver)) },
        );
    // Some servers send no response headers until they have read a message, and opening the call waits for
    // the headers, so the first request is queued before the call is opened.
    send(&outbound, request::MessageRequest::ListServices(LIST_ALL.to_owned())).await?;
    let mut grpc = Grpc::new(channel);
    grpc.ready().await.map_err(|e| Status::unavailable(format!("The server is not reachable: {e}")))?;
    let inbound = grpc
        .streaming(Request::new(stream), path.parse().expect("a constant path"), ProstCodec::default())
        .await?
        .into_inner();
    let mut talk = Conversation { outbound, inbound };

    let services = match talk.listen().await? {
        response::MessageResponse::ListServicesResponse(list) => {
            list.service.into_iter().map(|service| service.name).collect::<Vec<_>>()
        }
        response::MessageResponse::ErrorResponse(error) => {
            return Err(Status::new(Code::from(error.error_code), error.error_message));
        }
        response::MessageResponse::FileDescriptorResponse(_) => {
            return Err(Status::internal("The server answered a service list request with files."));
        }
    };

    let mut files: IndexMap<String, FileDescriptorProto> = IndexMap::new();
    for service in &services {
        let found = talk.files(request::MessageRequest::FileContainingSymbol(service.clone())).await?;
        for file in found.into_iter().flatten() {
            files.entry(file.name().to_owned()).or_insert(file);
        }
    }
    // A file the server did not send along with its dependents is asked for by name.
    let mut unavailable: HashSet<String> = HashSet::new();
    loop {
        let missing: Vec<String> = files
            .values()
            .flat_map(|file| file.dependency.iter())
            .filter(|name| !files.contains_key(*name) && !unavailable.contains(*name))
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            break;
        }
        for name in missing {
            match talk.files(request::MessageRequest::FileByFilename(name.clone())).await? {
                Some(found) => {
                    for file in found {
                        files.entry(file.name().to_owned()).or_insert(file);
                    }
                }
                None => {
                    unavailable.insert(name);
                }
            }
        }
    }

    let pool = build_pool(files.into_values().collect()).map_err(Status::internal)?;
    Ok(Reflected { pool, services })
}

/// Adds files to a pool. A file goes in once the files it imports are there; a file that is still refused after
/// everything else has been added is tried again, because some servers leave out the imports a file really has
/// (the error then names the type that is missing). The standard `google/protobuf` files are supplied when a
/// server leaves them out.
pub fn build_pool(files: Vec<FileDescriptorProto>) -> Result<DescriptorPool, String> {
    let mut pool = DescriptorPool::new();
    let well_known = DescriptorPool::global();
    let mut pending = files;
    let mut last_error = String::new();
    while !pending.is_empty() {
        let before = pending.len();
        let mut waiting = Vec::new();
        for file in std::mem::take(&mut pending) {
            let imports_present = file.dependency.iter().all(|dependency| pool.get_file_by_name(dependency).is_some());
            // A file with a missing import waits for its turn; the rest are tried now.
            if !imports_present {
                waiting.push(file);
                continue;
            }
            let mut attempt = pool.clone();
            match attempt.add_file_descriptor_proto(file.clone()) {
                Ok(()) => pool = attempt,
                Err(error) => {
                    last_error = error.to_string();
                    // The file may use types of files it does not list as imports: list them for it.
                    let mut widened = file.clone();
                    for known in pool.files() {
                        if known.name() != widened.name() && !widened.dependency.iter().any(|d| d == known.name()) {
                            widened.dependency.push(known.name().to_owned());
                        }
                    }
                    let mut retry = pool.clone();
                    match retry.add_file_descriptor_proto(widened) {
                        Ok(()) => pool = retry,
                        Err(_) => waiting.push(file),
                    }
                }
            }
        }
        pending = waiting;
        if pending.len() < before {
            continue;
        }
        let wanted: Vec<String> = pending
            .iter()
            .flat_map(|file| file.dependency.iter())
            .filter(|name| {
                pool.get_file_by_name(name).is_none() && !pending.iter().any(|file| file.name() == name.as_str())
            })
            .cloned()
            .collect();
        let mut supplied = false;
        for name in &wanted {
            if let Some(known) = well_known.get_file_by_name(name) {
                pool.add_file_descriptor_proto(known.file_descriptor_proto().clone())
                    .map_err(|e| format!("The schema is invalid: {e}"))?;
                supplied = true;
            }
        }
        if !supplied {
            return Err(if wanted.is_empty() {
                format!("The schema is invalid: {last_error}")
            } else {
                format!("The schema imports files that are not available: {}.", wanted.join(", "))
            });
        }
    }
    Ok(pool)
}

#[cfg(test)]
pub(super) mod server {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::Stream;
    use prost_reflect::DescriptorPool;
    use tonic::{
        Response, Status, Streaming,
        body::Body,
        server::{Grpc, NamedService, StreamingService},
    };
    use tonic_prost::ProstCodec;

    use super::{
        ErrorResponse, FileDescriptorResponse, ListServiceResponse, Message, ServerReflectionRequest,
        ServerReflectionResponse, ServiceResponse, request, response,
    };

    #[derive(Clone)]
    pub struct Reflection {
        pub pool: DescriptorPool,
        pub services: Vec<String>,
    }

    impl NamedService for Reflection {
        const NAME: &'static str = "grpc.reflection.v1.ServerReflection";
    }

    type Answers = Pin<Box<dyn Stream<Item = Result<ServerReflectionResponse, Status>> + Send>>;

    impl StreamingService<ServerReflectionRequest> for Reflection {
        type Response = ServerReflectionResponse;
        type ResponseStream = Answers;
        type Future = std::future::Ready<Result<Response<Answers>, Status>>;

        fn call(&mut self, request: tonic::Request<Streaming<ServerReflectionRequest>>) -> Self::Future {
            let this = self.clone();
            let mut inbound = request.into_inner();
            let stream =
                futures::stream::poll_fn(move |cx: &mut Context<'_>| match Pin::new(&mut inbound).poll_next(cx) {
                    Poll::Ready(Some(Ok(asked))) => Poll::Ready(Some(Ok(this.answer(asked)))),
                    Poll::Ready(Some(Err(status))) => Poll::Ready(Some(Err(status))),
                    Poll::Ready(None) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                });
            std::future::ready(Ok(Response::new(Box::pin(stream) as Answers)))
        }
    }

    impl Reflection {
        fn answer(&self, asked: ServerReflectionRequest) -> ServerReflectionResponse {
            let file_of = |name: &str| {
                let proto = self
                    .pool
                    .get_service_by_name(name)
                    .map(|service| service.parent_file())
                    .or_else(|| self.pool.get_file_by_name(name));
                match proto {
                    Some(file) => response::MessageResponse::FileDescriptorResponse(FileDescriptorResponse {
                        file_descriptor_proto: vec![file.file_descriptor_proto().encode_to_vec()],
                    }),
                    None => response::MessageResponse::ErrorResponse(ErrorResponse {
                        error_code: 5,
                        error_message: format!("{name} not found"),
                    }),
                }
            };
            let message = match asked.message_request.clone() {
                // Like some real servers, this one reads an empty list request as no request at all.
                Some(request::MessageRequest::ListServices(value)) if value.is_empty() => {
                    response::MessageResponse::ErrorResponse(ErrorResponse {
                        error_code: 3,
                        error_message: "empty request".into(),
                    })
                }
                Some(request::MessageRequest::ListServices(_)) => {
                    response::MessageResponse::ListServicesResponse(ListServiceResponse {
                        service: self.services.iter().map(|name| ServiceResponse { name: name.clone() }).collect(),
                    })
                }
                Some(request::MessageRequest::FileContainingSymbol(name))
                | Some(request::MessageRequest::FileByFilename(name)) => file_of(&name),
                None => response::MessageResponse::ErrorResponse(ErrorResponse {
                    error_code: 3,
                    error_message: "empty request".into(),
                }),
            };
            ServerReflectionResponse {
                valid_host: String::new(),
                original_request: Some(asked),
                message_response: Some(message),
            }
        }
    }

    impl tower_service::Service<http::Request<Body>> for Reflection {
        type Response = http::Response<Body>;
        type Error = std::convert::Infallible;
        type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: http::Request<Body>) -> Self::Future {
            let this = self.clone();
            Box::pin(async move {
                let mut grpc = Grpc::new(ProstCodec::<ServerReflectionResponse, ServerReflectionRequest>::default());
                Ok(grpc.streaming(this, request).await)
            })
        }
    }
}
