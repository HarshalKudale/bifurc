//! Compiling a saved `.proto` file into a descriptor pool, for servers that do not offer reflection.
//!
//! Imports are resolved against the other `.proto` files saved in the workspace (by name) and the standard
//! `google/protobuf` files. No `protoc` is involved.

use prost_reflect::DescriptorPool;
use protox::{
    Compiler,
    file::{ChainFileResolver, File, FileResolver, GoogleFileResolver},
};

use crate::{utils::json::str_of, workspace::fs::read_all_entities};

/// The workspace's saved proto files, as (name, source).
struct SavedProtos(Vec<(String, String)>);

impl SavedProtos {
    fn find(&self, name: &str) -> Option<&(String, String)> {
        let base = |n: &str| n.rsplit(['/', '\\']).next().unwrap_or(n).to_owned();
        self.0
            .iter()
            .find(|(saved, _)| saved == name)
            .or_else(|| self.0.iter().find(|(saved, _)| base(saved) == base(name)))
    }
}

impl FileResolver for SavedProtos {
    fn resolve_path(&self, path: &std::path::Path) -> Option<String> {
        let name = path.to_string_lossy();
        self.0.iter().find(|(saved, _)| *saved == name).map(|(saved, _)| saved.clone())
    }

    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        match self.find(name) {
            Some((_, source)) => File::from_source(name, source),
            None => Err(protox::Error::file_not_found(name)),
        }
    }
}

pub fn pool_from_saved(workspace_id: &str, proto_file_id: &str) -> Result<DescriptorPool, String> {
    let saved: Vec<(String, String, String)> = read_all_entities(workspace_id, "protoFiles")
        .iter()
        .map(|entity| {
            (
                str_of(entity, "id").to_owned(),
                str_of(entity, "name").replace('\\', "/"),
                str_of(entity, "content").to_owned(),
            )
        })
        .collect();
    let main = saved
        .iter()
        .find(|(id, ..)| id == proto_file_id)
        .ok_or_else(|| format!("The proto file \"{proto_file_id}\" is not saved in this workspace."))?;
    compile(&main.1, saved.iter().map(|(_, name, source)| (name.clone(), source.clone())).collect())
}

pub fn compile(main_name: &str, files: Vec<(String, String)>) -> Result<DescriptorPool, String> {
    let mut resolver = ChainFileResolver::new();
    resolver.add(SavedProtos(files));
    resolver.add(GoogleFileResolver::new());
    let mut compiler = Compiler::with_file_resolver(resolver);
    compiler.include_imports(true);
    compiler.open_file(main_name).map_err(|e| format!("The proto file does not compile: {e}"))?;
    Ok(compiler.descriptor_pool())
}
