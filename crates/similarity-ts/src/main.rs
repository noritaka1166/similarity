#![allow(clippy::uninlined_format_args)]

use clap::Parser;

mod check;
pub mod parallel;

#[derive(Parser)]
#[command(name = "similarity-ts")]
#[command(about = "TypeScript/JavaScript code similarity analyzer")]
#[command(version)]
struct Cli {
    /// Paths to analyze (files or directories)
    #[arg(default_value = ".")]
    paths: Vec<String>,

    /// Print code in output
    #[arg(short, long)]
    print: bool,

    /// Similarity threshold (0.0-1.0)
    #[arg(short, long, default_value = "0.87")]
    threshold: f64,

    /// Disable function similarity checking
    #[arg(long = "no-functions")]
    no_functions: bool,

    /// Enable type similarity checking (experimental)
    #[arg(long = "experimental-types")]
    types: bool,

    /// File extensions to check
    #[arg(short, long, value_delimiter = ',')]
    extensions: Option<Vec<String>>,

    /// Minimum lines for functions to be considered
    #[arg(short, long, default_value = "3")]
    min_lines: Option<u32>,

    /// Minimum tokens for functions to be considered
    #[arg(long)]
    min_tokens: Option<u32>,

    /// Rename cost for APTED algorithm
    #[arg(short, long, default_value = "0.3")]
    rename_cost: f64,

    /// Disable size penalty for very different sized functions
    #[arg(long)]
    no_size_penalty: bool,

    /// Filter functions by name (substring match)
    #[arg(long)]
    filter_function: Option<String>,

    /// Filter functions by body content (substring match)
    #[arg(long)]
    filter_function_body: Option<String>,

    /// Include both interfaces and type aliases
    #[arg(long)]
    include_types: bool,

    /// Only check type aliases (exclude interfaces)
    #[arg(long)]
    types_only: bool,

    /// Only check interfaces (exclude type aliases)
    #[arg(long)]
    interfaces_only: bool,

    /// Allow comparison between interfaces and type aliases
    #[arg(long, default_value = "true")]
    allow_cross_kind: bool,

    /// Weight for structural similarity (0.0-1.0)
    #[arg(long, default_value = "0.6")]
    structural_weight: f64,

    /// Weight for naming similarity (0.0-1.0)
    #[arg(long, default_value = "0.4")]
    naming_weight: f64,

    /// Include type literals (function return types, parameters, etc.)
    #[arg(long)]
    include_type_literals: bool,

    /// Disable fast mode with bloom filter pre-filtering
    #[arg(long = "no-fast")]
    no_fast: bool,

    /// Exclude directories matching the given patterns (can be specified multiple times)
    #[arg(long)]
    exclude: Vec<String>,

    /// Enable experimental overlap detection mode
    #[arg(long = "experimental-overlap")]
    overlap: bool,

    /// Minimum window size for overlap detection (number of nodes)
    #[arg(long, default_value = "8")]
    overlap_min_window: u32,

    /// Maximum window size for overlap detection (number of nodes)
    #[arg(long, default_value = "25")]
    overlap_max_window: u32,

    /// Size tolerance for overlap detection (0.0-1.0)
    #[arg(long, default_value = "0.25")]
    overlap_size_tolerance: f64,

    /// Exit with code 1 if duplicates are found
    #[arg(long)]
    fail_on_duplicates: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let functions_enabled = !cli.no_functions;
    let types_enabled = cli.types;
    let overlap_enabled = cli.overlap;

    // Validate that at least one analyzer is enabled
    if !functions_enabled && !types_enabled && !overlap_enabled {
        eprintln!("Error: At least one analyzer must be enabled. Use --types to enable type checking, --overlap for overlap detection, or remove --no-functions.");
        return Err(anyhow::anyhow!("No analyzer enabled"));
    }

    // Handle mutual exclusion of min_lines and min_tokens
    let (min_lines, min_tokens) = match (cli.min_lines, cli.min_tokens) {
        (Some(_), Some(tokens)) => {
            eprintln!(
                "Warning: Both --min-lines and --min-tokens specified. Using --min-tokens={}",
                tokens
            );
            (None, Some(tokens))
        }
        (lines, tokens) => (lines, tokens),
    };

    println!("Analyzing code similarity...\n");

    let separator = "-".repeat(60);
    let mut total_duplicates = 0;

    // Run functions analysis if enabled
    if functions_enabled {
        println!("=== Function Similarity ===");
        let duplicate_count = check::check_paths(
            cli.paths.clone(),
            cli.threshold,
            cli.rename_cost,
            cli.extensions.as_ref(),
            min_lines.unwrap_or(3),
            min_tokens,
            cli.no_size_penalty,
            cli.print,
            !cli.no_fast,
            cli.filter_function.as_ref(),
            cli.filter_function_body.as_ref(),
            &cli.exclude,
        )?;
        total_duplicates += duplicate_count;
    }

    // Run types analysis if enabled
    if types_enabled && functions_enabled {
        println!("\n{}\n", separator);
    }

    if types_enabled {
        println!("=== Type Similarity ===");
        let type_duplicate_count = check_types(
            cli.paths.clone(),
            cli.threshold,
            cli.extensions.as_ref(),
            cli.print,
            cli.include_types,
            cli.types_only,
            cli.interfaces_only,
            cli.allow_cross_kind,
            cli.structural_weight,
            cli.naming_weight,
            cli.include_type_literals,
            &cli.exclude,
        )?;
        total_duplicates += type_duplicate_count;
    }

    // Run overlap analysis if enabled
    if overlap_enabled && (functions_enabled || types_enabled) {
        println!("\n{}\n", separator);
    }

    if overlap_enabled {
        println!("=== Overlap Detection ===");
        let overlap_duplicate_count = check_overlaps(
            cli.paths,
            cli.threshold,
            cli.extensions.as_ref(),
            cli.print,
            cli.overlap_min_window,
            cli.overlap_max_window,
            cli.overlap_size_tolerance,
            &cli.exclude,
        )?;
        total_duplicates += overlap_duplicate_count;
    }

    // Exit with code 1 if duplicates found and --fail-on-duplicates is set
    if cli.fail_on_duplicates && total_duplicates > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn create_exclude_matcher(exclude_patterns: &[String]) -> Option<globset::GlobSet> {
    if exclude_patterns.is_empty() {
        return None;
    }

    let mut builder = globset::GlobSetBuilder::new();
    for pattern in exclude_patterns {
        if let Ok(glob) = globset::Glob::new(pattern) {
            builder.add(glob);
        } else {
            eprintln!("Warning: Invalid glob pattern: {}", pattern);
        }
    }

    builder.build().ok()
}

#[allow(clippy::too_many_arguments)]
fn check_types(
    paths: Vec<String>,
    threshold: f64,
    extensions: Option<&Vec<String>>,
    print: bool,
    _include_types: bool,
    types_only: bool,
    interfaces_only: bool,
    allow_cross_kind: bool,
    structural_weight: f64,
    naming_weight: f64,
    include_type_literals: bool,
    exclude_patterns: &[String],
) -> anyhow::Result<usize> {
    use ignore::WalkBuilder;
    use similarity_core::{
        extract_type_literals_from_code, extract_types_from_code, find_similar_type_literals,
        find_similar_types, TypeComparisonOptions, TypeKind,
    };
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    let default_extensions = vec!["ts", "tsx", "mts", "cts"];
    let exts: Vec<&str> =
        extensions.map_or(default_extensions, |v| v.iter().map(String::as_str).collect());

    let exclude_matcher = create_exclude_matcher(exclude_patterns);
    let mut files = Vec::new();
    let mut visited = HashSet::new();

    // Process each path
    for path_str in &paths {
        let path = Path::new(path_str);

        if path.is_file() {
            // If it's a file, check extension and add it
            if let Some(ext) = path.extension() {
                if let Some(ext_str) = ext.to_str() {
                    if exts.contains(&ext_str) {
                        if let Ok(canonical) = path.canonicalize() {
                            if visited.insert(canonical.clone()) {
                                files.push(path.to_path_buf());
                            }
                        }
                    }
                }
            }
        } else if path.is_dir() {
            // If it's a directory, walk it respecting .gitignore
            let walker = WalkBuilder::new(path).follow_links(false).build();

            for entry in walker {
                let entry = entry?;
                let entry_path = entry.path();

                // Skip if not a file
                if !entry_path.is_file() {
                    continue;
                }

                // Check if path should be excluded
                if let Some(ref matcher) = exclude_matcher {
                    if matcher.is_match(entry_path) {
                        continue;
                    }
                }

                // Check extension
                if let Some(ext) = entry_path.extension() {
                    if let Some(ext_str) = ext.to_str() {
                        if exts.contains(&ext_str) {
                            // Get canonical path to avoid duplicates
                            if let Ok(canonical) = entry_path.canonicalize() {
                                if visited.insert(canonical.clone()) {
                                    files.push(entry_path.to_path_buf());
                                }
                            }
                        }
                    }
                }
            }
        } else {
            eprintln!("Warning: Path not found: {}", path_str);
        }
    }

    if files.is_empty() {
        println!("No TypeScript files found in specified paths");
        return Ok(0);
    }

    println!("Checking {} files for similar types...\n", files.len());

    // Extract types from all files
    let mut all_types = Vec::new();
    let mut all_type_literals = Vec::new();

    for file in &files {
        match fs::read_to_string(file) {
            Ok(content) => {
                let file_str = file.to_string_lossy();

                // Extract regular types
                match extract_types_from_code(&content, &file_str) {
                    Ok(mut types) => {
                        // Filter types based on command line options
                        if types_only {
                            types.retain(|t| t.kind == TypeKind::TypeAlias);
                        } else if interfaces_only {
                            types.retain(|t| t.kind == TypeKind::Interface);
                        }
                        all_types.extend(types);
                    }
                    Err(e) => {
                        // Skip files with parse errors silently
                        if !e.contains("Parse errors:") {
                            eprintln!("Error in {}: {}", file.display(), e);
                        }
                    }
                }

                // Extract type literals if requested
                if include_type_literals {
                    match extract_type_literals_from_code(&content, &file_str) {
                        Ok(type_literals) => {
                            all_type_literals.extend(type_literals);
                        }
                        Err(e) => {
                            // Skip files with parse errors silently
                            if !e.contains("Parse errors:") {
                                eprintln!("Error in {}: {}", file.display(), e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading {}: {}", file.display(), e);
            }
        }
    }

    if all_types.is_empty() && all_type_literals.is_empty() {
        println!("No type definitions or type literals found!");
        return Ok(0);
    }

    println!("Found {} type definitions", all_types.len());
    if include_type_literals {
        println!("Found {} type literals", all_type_literals.len());
    }

    // Set up comparison options
    let options = TypeComparisonOptions {
        allow_cross_kind_comparison: allow_cross_kind,
        structural_weight,
        naming_weight,
        ..Default::default()
    };

    // Validate weights
    if (structural_weight + naming_weight - 1.0).abs() > 0.001 {
        eprintln!("Warning: structural_weight + naming_weight should equal 1.0");
    }

    // Find similar types across all files
    let similar_pairs = find_similar_types(&all_types, threshold, &options);

    // Find type literals similar to type definitions
    let type_literal_pairs = if include_type_literals {
        find_similar_type_literals(&all_type_literals, &all_types, threshold, &options)
    } else {
        Vec::new()
    };

    if similar_pairs.is_empty() && type_literal_pairs.is_empty() {
        println!("\nNo similar types found!");
    } else {
        if !similar_pairs.is_empty() {
            println!("\nSimilar types found:");
            println!("{}", "-".repeat(60));

            for pair in &similar_pairs {
                // Get relative paths
                let relative_path1 = get_relative_path(&pair.type1.file_path);
                let relative_path2 = get_relative_path(&pair.type2.file_path);

                println!(
                    "\nSimilarity: {:.2}% (structural: {:.2}%, naming: {:.2}%)",
                    pair.result.similarity * 100.0,
                    pair.result.structural_similarity * 100.0,
                    pair.result.naming_similarity * 100.0
                );
                println!(
                    "  {}:{} | L{}-{} similar-type: {} ({})",
                    relative_path1,
                    pair.type1.start_line,
                    pair.type1.start_line,
                    pair.type1.end_line,
                    pair.type1.name,
                    format_type_kind(&pair.type1.kind)
                );
                println!(
                    "  {}:{} | L{}-{} similar-type: {} ({})",
                    relative_path2,
                    pair.type2.start_line,
                    pair.type2.start_line,
                    pair.type2.end_line,
                    pair.type2.name,
                    format_type_kind(&pair.type2.kind)
                );

                if print {
                    show_type_details(&pair.type1);
                    show_type_details(&pair.type2);
                    show_comparison_details(&pair.result);
                }
            }

            println!("\nTotal similar type pairs found: {}", similar_pairs.len());
        }

        if !type_literal_pairs.is_empty() {
            println!("\nType literals similar to type definitions:");
            println!("{}", "-".repeat(60));

            for pair in &type_literal_pairs {
                let literal_path = get_relative_path(&pair.type_literal.file_path);
                let def_path = get_relative_path(&pair.type_definition.file_path);

                println!(
                    "\nSimilarity: {:.2}% (structural: {:.2}%, naming: {:.2}%)",
                    pair.result.similarity * 100.0,
                    pair.result.structural_similarity * 100.0,
                    pair.result.naming_similarity * 100.0
                );
                println!(
                    "  {}:{} | L{} similar-type-literal: {}",
                    literal_path,
                    pair.type_literal.start_line,
                    pair.type_literal.start_line,
                    pair.type_literal.name
                );
                println!(
                    "  {}:{} | L{}-{} similar-type: {} ({})",
                    def_path,
                    pair.type_definition.start_line,
                    pair.type_definition.start_line,
                    pair.type_definition.end_line,
                    pair.type_definition.name,
                    format_type_kind(&pair.type_definition.kind)
                );

                if print {
                    show_type_literal_details(&pair.type_literal);
                    show_type_details(&pair.type_definition);
                    show_comparison_details(&pair.result);
                }
            }

            println!("\nTotal type literal pairs found: {}", type_literal_pairs.len());
        }
    }

    Ok(similar_pairs.len() + type_literal_pairs.len())
}

fn get_relative_path(file_path: &str) -> String {
    if let Ok(current_dir) = std::env::current_dir() {
        std::path::Path::new(file_path)
            .strip_prefix(&current_dir)
            .unwrap_or(std::path::Path::new(file_path))
            .to_string_lossy()
            .to_string()
    } else {
        file_path.to_string()
    }
}

fn format_type_kind(kind: &similarity_core::TypeKind) -> &'static str {
    match kind {
        similarity_core::TypeKind::Interface => "interface",
        similarity_core::TypeKind::TypeAlias => "type",
        similarity_core::TypeKind::TypeLiteral => "type literal",
    }
}

fn show_type_details(type_def: &similarity_core::TypeDefinition) {
    println!("\n\x1b[36m--- {} ({}) ---\x1b[0m", type_def.name, format_type_kind(&type_def.kind));

    if !type_def.generics.is_empty() {
        println!("Generics: <{}>", type_def.generics.join(", "));
    }

    if !type_def.extends.is_empty() {
        println!("Extends: {}", type_def.extends.join(", "));
    }

    if !type_def.properties.is_empty() {
        println!("Properties:");
        for prop in &type_def.properties {
            let modifiers = if prop.readonly { "readonly " } else { "" };
            let optional = if prop.optional { "?" } else { "" };
            println!("  {}{}{}: {}", modifiers, prop.name, optional, prop.type_annotation);
        }
    }
}

fn show_type_literal_details(type_literal: &similarity_core::TypeLiteralDefinition) {
    println!("\n\x1b[36m--- {} (type literal) ---\x1b[0m", type_literal.name);

    println!("Context: {}", format_type_literal_context(&type_literal.context));

    if !type_literal.properties.is_empty() {
        println!("Properties:");
        for prop in &type_literal.properties {
            let modifiers = if prop.readonly { "readonly " } else { "" };
            let optional = if prop.optional { "?" } else { "" };
            println!("  {}{}{}: {}", modifiers, prop.name, optional, prop.type_annotation);
        }
    }
}

fn format_type_literal_context(context: &similarity_core::TypeLiteralContext) -> String {
    match context {
        similarity_core::TypeLiteralContext::FunctionReturn(name) => {
            format!("Function '{}' return type", name)
        }
        similarity_core::TypeLiteralContext::FunctionParameter(func_name, param_name) => {
            format!("Function '{}' parameter '{}'", func_name, param_name)
        }
        similarity_core::TypeLiteralContext::VariableDeclaration(name) => {
            format!("Variable '{}' type annotation", name)
        }
        similarity_core::TypeLiteralContext::ArrowFunctionReturn(name) => {
            format!("Arrow function '{}' return type", name)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_overlaps(
    paths: Vec<String>,
    threshold: f64,
    extensions: Option<&Vec<String>>,
    print: bool,
    min_window_size: u32,
    max_window_size: u32,
    size_tolerance: f64,
    exclude_patterns: &[String],
) -> anyhow::Result<usize> {
    use ignore::WalkBuilder;
    use similarity_core::{find_overlaps_across_files, OverlapOptions};
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::Path;

    let default_extensions = vec!["js", "ts", "jsx", "tsx", "mjs", "mts", "cjs", "cts"];
    let exts: Vec<&str> =
        extensions.map_or(default_extensions, |v| v.iter().map(String::as_str).collect());

    let exclude_matcher = create_exclude_matcher(exclude_patterns);
    let mut files = Vec::new();
    let mut visited = HashSet::new();

    // Process each path
    for path_str in &paths {
        let path = Path::new(path_str);

        if path.is_file() {
            // If it's a file, check extension and add it
            if let Some(ext) = path.extension() {
                if let Some(ext_str) = ext.to_str() {
                    if exts.contains(&ext_str) {
                        if let Ok(canonical) = path.canonicalize() {
                            if visited.insert(canonical.clone()) {
                                files.push(path.to_path_buf());
                            }
                        }
                    }
                }
            }
        } else if path.is_dir() {
            // If it's a directory, walk it respecting .gitignore
            let walker = WalkBuilder::new(path).follow_links(false).build();

            for entry in walker {
                let entry = entry?;
                let entry_path = entry.path();

                // Skip if not a file
                if !entry_path.is_file() {
                    continue;
                }

                // Check if path should be excluded
                if let Some(ref matcher) = exclude_matcher {
                    if matcher.is_match(entry_path) {
                        continue;
                    }
                }

                // Check extension
                if let Some(ext) = entry_path.extension() {
                    if let Some(ext_str) = ext.to_str() {
                        if exts.contains(&ext_str) {
                            // Get canonical path to avoid duplicates
                            if let Ok(canonical) = entry_path.canonicalize() {
                                if visited.insert(canonical.clone()) {
                                    files.push(entry_path.to_path_buf());
                                }
                            }
                        }
                    }
                }
            }
        } else {
            eprintln!("Warning: Path not found: {}", path_str);
        }
    }

    if files.is_empty() {
        println!("No JavaScript/TypeScript files found in specified paths");
        return Ok(0);
    }

    println!("Checking {} files for overlapping code...\n", files.len());

    // Read all file contents
    let mut file_contents = HashMap::new();
    for file in &files {
        match fs::read_to_string(file) {
            Ok(content) => {
                let file_str = file.to_string_lossy().to_string();
                file_contents.insert(file_str, content);
            }
            Err(e) => {
                eprintln!("Error reading {}: {}", file.display(), e);
            }
        }
    }

    // Set up overlap options
    let options = OverlapOptions { min_window_size, max_window_size, threshold, size_tolerance };

    // Find overlaps
    let overlaps = find_overlaps_across_files(&file_contents, &options)?;

    if overlaps.is_empty() {
        println!("\nNo code overlaps found!");
    } else {
        println!("\nCode overlaps found:");
        println!("{}", "-".repeat(60));

        for overlap_with_files in &overlaps {
            let overlap = &overlap_with_files.overlap;
            let source_path = get_relative_path(&overlap_with_files.source_file);
            let target_path = get_relative_path(&overlap_with_files.target_file);

            println!(
                "\nSimilarity: {:.2}% | {} nodes | {}",
                overlap.similarity * 100.0,
                overlap.node_count,
                overlap.node_type
            );
            println!(
                "  {}:{} | L{}-{} in function: {}",
                source_path,
                overlap.source_lines.0,
                overlap.source_lines.0,
                overlap.source_lines.1,
                overlap.source_function
            );
            println!(
                "  {}:{} | L{}-{} in function: {}",
                target_path,
                overlap.target_lines.0,
                overlap.target_lines.0,
                overlap.target_lines.1,
                overlap.target_function
            );

            if print {
                // Extract and display the overlapping code
                if let Some(source_content) = file_contents.get(&overlap_with_files.source_file) {
                    if let Some(target_content) = file_contents.get(&overlap_with_files.target_file)
                    {
                        println!("\n\x1b[36m--- Source Code ---\x1b[0m");
                        if let Ok(source_segment) = extract_code_lines(
                            source_content,
                            overlap.source_lines.0,
                            overlap.source_lines.1,
                        ) {
                            println!("{}", source_segment);
                        }

                        println!("\n\x1b[36m--- Target Code ---\x1b[0m");
                        if let Ok(target_segment) = extract_code_lines(
                            target_content,
                            overlap.target_lines.0,
                            overlap.target_lines.1,
                        ) {
                            println!("{}", target_segment);
                        }
                    }
                }
            }
        }

        println!("\nTotal overlaps found: {}", overlaps.len());
    }

    Ok(overlaps.len())
}

fn extract_code_lines(code: &str, start_line: u32, end_line: u32) -> Result<String, String> {
    let lines: Vec<_> = code.lines().collect();

    if start_line as usize > lines.len() || end_line as usize > lines.len() {
        return Err("Line numbers out of bounds".to_string());
    }

    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());

    Ok(lines[start..end].join("\n"))
}

fn show_comparison_details(result: &similarity_core::TypeComparisonResult) {
    if !result.differences.missing_properties.is_empty() {
        println!("Missing properties: {}", result.differences.missing_properties.join(", "));
    }

    if !result.differences.extra_properties.is_empty() {
        println!("Extra properties: {}", result.differences.extra_properties.join(", "));
    }

    if !result.differences.type_mismatches.is_empty() {
        println!("Type mismatches:");
        for mismatch in &result.differences.type_mismatches {
            println!("  {}: {} vs {}", mismatch.property, mismatch.type1, mismatch.type2);
        }
    }

    if !result.differences.optionality_differences.is_empty() {
        println!(
            "Optionality differences: {}",
            result.differences.optionality_differences.join(", ")
        );
    }
}
