//! LLM prompt templates for the LongMemEval benchmark pipeline.
//!
//! Provides prompts for answer generation (with memory context + timestamps)
//! and GPT-4o judge evaluation (standard and abstention variants).

use crate::store::Memory;

/// Build the answer generation prompt. Includes retrieved memories with timestamps
/// for temporal reasoning support.
pub fn build_answer_prompt(
    question: &str,
    question_date: &str,
    retrieved_memories: &[Memory],
) -> String {
    // Sort memories chronologically for temporal reasoning accuracy
    let mut sorted_memories: Vec<&Memory> = retrieved_memories.iter().collect();
    sorted_memories.sort_by_key(|m| m.created_at);

    let context = sorted_memories
        .iter()
        .enumerate()
        .map(|(i, m)| {
            format!(
                "[Memory {}] (created: {})\n{}",
                i + 1,
                m.created_at.format("%Y-%m-%d %H:%M"),
                m.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "You are a helpful assistant with access to a user's conversation history stored as memories.\n\n\
         Today's date: {question_date}\n\n\
         Conversation memories in chronological order:\n{context}\n\n\
         Question: {question}\n\n\
         Instructions:\n\
         1. Carefully read ALL memories above — the answer may require combining facts from multiple conversations.\n\
         2. For counting questions (\"how many\"), be STRICT about what you count. Only count items that directly and \
         specifically match the question criteria. Do not count tangentially related items, and do not count the same \
         item twice even if mentioned in multiple memories. List each qualifying item with its source memory, then total.\n\
         3. For calculations (costs, time, distances), extract the specific numbers from the memories and show your arithmetic.\n\
         4. For date/duration questions, identify the specific dates and calculate step by step. IMPORTANT: when a memory \
         uses relative time expressions like \"a month ago\" or \"last week\", resolve them relative to THAT MEMORY'S \
         creation date, not today's date.\n\
         5. When the same topic appears in multiple memories, the MOST RECENT memory supersedes earlier ones — \
         do NOT add or combine them. For example: if Memory 3 says \"I have 17 postcards\" and Memory 7 says \
         \"I now have 25 postcards\", the answer is 25 (not 17+25=42). Similarly, if Memory 3 says \"I keep my shoes \
         under the bed\" and Memory 7 says \"I moved my shoes to the closet\", the current answer is the closet.\n\
         6. If the memories contain the specific data needed to answer — even if it requires arithmetic, combining facts, \
         or simple inference — compute and state the answer rather than hedging. However, if the question asks about \
         something genuinely not discussed (e.g., a role or event never mentioned), or makes a false assumption, \
         say \"I don't have that information in my memory.\" The key distinction: do the memories contain the actual \
         facts needed? If yes, answer. If the question assumes something not in the memories, decline.\n\n\
         Be concise and direct."
    )
}

/// Build the standard judge prompt for non-abstention questions.
/// GPT-4o evaluates whether the hypothesis correctly answers the question given ground truth.
pub fn build_judge_prompt(question: &str, ground_truth: &str, hypothesis: &str) -> String {
    format!(
        "You are evaluating whether a chat assistant correctly answered a question based on its conversation memory.\n\n\
         Question: {question}\n\n\
         Ground truth answer: {ground_truth}\n\n\
         Assistant's response: {hypothesis}\n\n\
         Does the assistant's response correctly contain the ground truth answer? \
         The response doesn't need to match word-for-word, but must convey the same factual information. \
         Answer with only 'yes' or 'no'."
    )
}

/// Build the abstention judge prompt. Checks whether the model correctly identified
/// that it cannot answer the question (the question has a false premise or asks about
/// information not in the conversation history).
pub fn build_abstention_judge_prompt(question: &str, hypothesis: &str) -> String {
    format!(
        "You are evaluating whether a chat assistant correctly identified that a question \
         cannot be answered from its conversation history. The question has a false premise \
         or asks about information not in the history.\n\n\
         Question: {question}\n\n\
         Assistant's response: {hypothesis}\n\n\
         Did the assistant appropriately indicate it cannot answer, express uncertainty, \
         or decline to provide a specific answer? Answer with only 'yes' or 'no'."
    )
}
