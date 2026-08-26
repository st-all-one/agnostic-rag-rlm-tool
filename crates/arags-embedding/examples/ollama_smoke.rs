use arags_embedding::embedder::{EmbeddingConfig, EmbeddingModel, build_embedder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = EmbeddingConfig {
        model: EmbeddingModel::Ollama,
        model_dir: None,
        quantization: Default::default(),
        ollama_url: std::env::var("ARAGS_OLLAMA_URL").ok(),
        ollama_model: std::env::var("ARAGS_OLLAMA_MODEL").ok(),
        #[cfg(feature = "llamacpp")]
        llama_cpp_model: None,
        #[cfg(feature = "llamacpp")]
        llama_cpp_gpu_layers: 99,
    };
    let emb = build_embedder(&cfg)?;
    println!("name={} dims={}", emb.name(), emb.dimensions());

    let texts: Vec<String> = (0..32)
        .map(|i| format!("fn process_{i}() {{ let x = {i}; return x; }}"))
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();

    let t = std::time::Instant::now();
    let v = emb.embed_batch(&refs)?;
    let dt = t.elapsed().as_secs_f64();
    let n = v.len() as u64;
    println!(
        "{} embeddings in {:.3}s ({:.1} chunks/s, ~{:.0} ms/chunk)",
        n,
        dt,
        n as f64 / dt,
        dt * 1000.0 / n as f64
    );
    println!("first vector len = {}", v.first().map_or(0, Vec::len));
    Ok(())
}
