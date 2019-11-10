//! See [README.md](https://github.com/slava-sh/rust-bundler/blob/master/README.md)
extern crate cargo_metadata;
extern crate quote;
extern crate rustfmt;
extern crate syn;

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Sink};
use std::mem;
use std::path::Path;

use cargo_metadata::{Metadata, MetadataCommand, Node, Package, PackageId, Target};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;

/// Creates a single-source-file version of a Cargo package.
pub fn bundle<P: AsRef<Path>>(package_path: P, skip_extern_crate_expansion: &HashSet<String>) -> String {
	let manifest_path = package_path.as_ref().join("Cargo.toml");
	let metadata = MetadataCommand::new()
		.manifest_path(&manifest_path)
		.exec()
		.expect("failed to obtain cargo metadata");
	let metadata_resolve = metadata.resolve.as_ref().unwrap();
	let root_id = metadata_resolve.root.as_ref().unwrap();

	let root_package = metadata
		.packages
		.iter()
		.find(|package| &package.id == root_id)
		.expect("Could not find root package");

	let target = root_package
		.targets
		.iter()
		.find(|t| target_is(t, "bin"))
		.or_else(|| root_package.targets.iter().find(|t| target_is(t, "lib")))
		.expect("Could not find target");

	let resolve_node = metadata_resolve
		.nodes
		.iter()
		.find(|node| &node.id == root_id)
		.expect("Could not find node");

	let base_path = Path::new(&target.src_path)
		.parent()
		.expect("lib.src_path has no parent");

	eprintln!("expanding target {}", target.src_path.to_string_lossy());
	let code = read_file(&Path::new(&target.src_path)).expect("failed to read target source");
	let mut file = syn::parse_file(&code).expect("failed to parse target source");
	Expander {
		base_path: &base_path,
		metadata: &metadata,
		package: root_package,
		target,
		resolve_node,
		skip_extern_crate_expansion,
		depth: 0,
	}
	.visit_file_mut(&mut file);
	let code = file.into_token_stream().to_string();
	prettify(code)
}

fn target_is(target: &Target, target_kind: &str) -> bool {
	target.kind.iter().any(|kind| kind == target_kind)
}

fn package_target<'a>(package: &'a Package, target_type: &str) -> &'a Target {
	package
		.targets
		.iter()
		.find(|t| target_is(t, target_type))
		.expect(&format!(
			"Could not find target of type {} in package {}",
			target_type, package.name
		))
}

struct Expander<'a> {
	base_path: &'a Path,
	metadata: &'a Metadata,
	package: &'a Package,
	target: &'a Target,
	resolve_node: &'a Node,
	skip_extern_crate_expansion: &'a HashSet<String>,
	depth: usize,
}

impl<'a> Expander<'a> {
	fn expand_items(&self, items: &mut Vec<syn::Item>) {
		self.expand_extern_crate(items);
		self.expand_use_path(items);
	}

	fn expand_extern_crate(&self, items: &mut Vec<syn::Item>) {
		let mut new_items = Vec::with_capacity(items.len());
		for item in items.drain(..) {
			match is_extern_crate(&item) {
				Some(extern_crate_ident)
					if !self
						.skip_extern_crate_expansion
						.contains(&extern_crate_ident.to_string()) =>
				{
					let extern_crate_name = extern_crate_ident.to_string();
					eprintln!(
						"expanding crate {} in {} at depth {}",
						&extern_crate_name,
						self.base_path.to_str().unwrap(),
						self.depth,
					);
					let root_id = self.root_id();
					let is_root_lib_expand = self.is_root() && self.root_lib_name() == &extern_crate_name;
					let crate_id = if is_root_lib_expand {
						root_id
					} else {
						if let Some(dep) = self.resolve_node.deps.iter().find(|dep| dep.name == extern_crate_name) {
							&dep.pkg
						} else {
							eprintln!(
								"Warning: could not find dep {} in crate {}",
								extern_crate_name, self.package.name
							);
							continue;
						}
					};
					let package = &self.package_by_id(crate_id);
					let target = package_target(package, "lib");
					let code = read_file(&target.src_path).expect("failed to read at lib src path");
					let mut lib = syn::parse_file(&code).expect("failed to parse at lib src path");
					Expander {
						base_path: &target.src_path.parent().unwrap(),
						metadata: self.metadata,
						package,
						target,
						resolve_node: self
							.metadata
							.resolve
							.as_ref()
							.unwrap()
							.nodes
							.iter()
							.find(|n| &n.id == crate_id)
							.expect("Could not find resolve_node"),
						skip_extern_crate_expansion: self.skip_extern_crate_expansion,
						depth: self.depth + 1,
					}
					.visit_file_mut(&mut lib);
					if is_root_lib_expand {
						new_items.extend(lib.items);
					} else {
						new_items.push(syn::Item::Mod(syn::ItemMod {
							attrs: Vec::new(),
							vis: syn::Visibility::Public(syn::VisPublic {
								pub_token: Default::default(),
							}),
							mod_token: syn::token::Mod::default(),
							ident: extern_crate_ident.clone(),
							content: Some((Default::default(), lib.items)),
							semi: None,
						}))
					}
				}
				_ => new_items.push(item),
			}
		}
		*items = new_items;
	}

	fn expand_use_path(&self, items: &mut Vec<syn::Item>) {
		if self.is_root() {
			let root_name = self.root_lib_name();
			items.retain(|i| !is_use_path(i, root_name));
		}
	}

	fn expand_mods(&self, item: &mut syn::ItemMod) {
		if item.content.is_some() {
			return;
		}
		let name = item.ident.to_string();
		let other_base_path = self.base_path.join(&name);
		let (base_path, code) = vec![
			(self.base_path, format!("{}.rs", name)),
			(&other_base_path, String::from("mod.rs")),
		]
		.into_iter()
		.flat_map(|(base_path, file_name)| read_file(&base_path.join(file_name)).map(|code| (base_path, code)))
		.next()
		.expect("mod not found");
		eprintln!(
			"expanding mod {} in {} at depth {}",
			name,
			base_path.to_str().unwrap(),
			self.depth
		);
		let mut file = syn::parse_file(&code).expect("failed to parse file");
		Expander {
			base_path,
			metadata: self.metadata,
			package: self.package,
			target: self.target,
			resolve_node: self.resolve_node,
			skip_extern_crate_expansion: self.skip_extern_crate_expansion,
			depth: self.depth,
		}
		.visit_file_mut(&mut file);
		item.content = Some((Default::default(), file.items));
	}

	fn expand_crate_path(&mut self, path: &mut syn::Path) {
		if self.is_root() && path_starts_with(path, self.root_lib_name()) {
			let new_segments = mem::replace(&mut path.segments, Punctuated::new())
				.into_pairs()
				.skip(1)
				.collect();
			path.segments = new_segments;
		}
	}

	fn root_lib_name(&self) -> &str {
		let root_package = self.package_by_id(self.root_id());
		let lib_package = root_package.targets.iter().find(|t| target_is(t, "lib"));
		lib_package.map(|p| &p.name).unwrap_or(&root_package.name)
	}
	fn package_by_id(&self, package_id: &PackageId) -> &Package {
		self.metadata
			.packages
			.iter()
			.find(|package| &package.id == package_id)
			.expect("Could not find package by id")
	}
	fn root_id(&self) -> &PackageId {
		self.metadata.resolve.as_ref().unwrap().root.as_ref().unwrap()
	}
	fn is_root(&self) -> bool {
		self.root_id() == &self.package.id
	}
}

impl<'a> VisitMut for Expander<'a> {
	fn visit_file_mut(&mut self, file: &mut syn::File) {
		for it in &mut file.attrs {
			self.visit_attribute_mut(it)
		}
		self.expand_items(&mut file.items);
		for it in &mut file.items {
			self.visit_item_mut(it)
		}
	}

	fn visit_item_mod_mut(&mut self, item: &mut syn::ItemMod) {
		for it in &mut item.attrs {
			self.visit_attribute_mut(it)
		}
		self.visit_visibility_mut(&mut item.vis);
		self.visit_ident_mut(&mut item.ident);
		self.expand_mods(item);
		if let Some(ref mut it) = item.content {
			for it in &mut (it).1 {
				self.visit_item_mut(it);
			}
		}
	}

	fn visit_path_mut(&mut self, path: &mut syn::Path) {
		self.expand_crate_path(path);
		for mut el in Punctuated::pairs_mut(&mut path.segments) {
			let it = el.value_mut();
			self.visit_path_segment_mut(it)
		}
	}
}

fn is_extern_crate(item: &syn::Item) -> Option<&syn::Ident> {
	match item {
		syn::Item::ExternCrate(item) => Some(&item.ident),
		_ => None,
	}
}

fn path_starts_with(path: &syn::Path, segment: &str) -> bool {
	path.segments.first().map_or(false, |el| el.ident == segment)
}

fn is_use_path(item: &syn::Item, first_segment: &str) -> bool {
	if let syn::Item::Use(ref item) = *item {
		if let syn::UseTree::Path(ref path) = item.tree {
			if path.ident == first_segment {
				return true;
			}
		}
	}
	false
}

fn read_file(path: &Path) -> Option<String> {
	let mut buf = String::new();
	File::open(path).ok()?.read_to_string(&mut buf).ok()?;
	Some(buf)
}

fn prettify(code: String) -> String {
	let code = code.replace(" try ! ", "r#try!");
	let code_clone = code.clone();
	std::panic::catch_unwind(|| {
		match rustfmt::format_input::<Sink>(rustfmt::Input::Text(code.clone()), &Default::default(), None)
			.expect("rustfmt failed")
			.1
			.first()
		{
			Some((_path, code)) => code.to_string(),
			None => {
				eprintln!("rustfmt failed");
				code
			}
		}
	})
	.unwrap_or(code_clone)
}
