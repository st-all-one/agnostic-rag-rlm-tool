use std::path::Path;
use std::time::Instant;

use arags_embedding::embedder::llama_cpp::LlamaCppEmbedder;
use arags_embedding::embedder::Embedder;

fn main() {
    let model = std::env::args().nth(1).expect("usage: llamacpp_bench <gguf> [gpu_layers]");
    let gpu: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(99);

    let emb = LlamaCppEmbedder::new(Path::new(&model), gpu, 512).expect("load gguf");
    println!("dims = {}", emb.dimensions());

    // Synthetic chunk sized to ~480 tokens (fits n_ctx=512) by repeating a
    // representative code line until the tokenizer reaches the target.
    let base = "fn process(items: &[Item]) -> Vec<Out> { let mut v = Vec::new(); for it in items { v.push(transform(it)); } v }\n";
    let target_tokens = 480usize;
    let mut chunk = String::new();
    while emb.count_tokens(&chunk) < target_tokens {
        chunk.push_str(base);
    }
    let actual = emb.count_tokens(&chunk);
    println!("synthetic chunk ~ {actual} tokens");

    let batch_size = 32usize;
    let n_batches = 50usize;
    let texts: Vec<String> = (0..batch_size).map(|_| chunk.clone()).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();

    // Warmup (GPU upload + graph compile).
    emb.embed_batch(&refs).expect("warmup");

    let t = Instant::now();
    for _ in 0..n_batches {
        emb.embed_batch(&refs).expect("embed");
    }
    let dt = t.elapsed().as_secs_f64();
    let total = n_batches * batch_size;
    println!(
        "embedded {total} chunks (~512 tok each) in {dt:.3}s => {:.1} chunks/s ({:.2} ms/chunk)",
        total as f64 / dt,
        dt * 1000.0 / total as f64
    );
}
