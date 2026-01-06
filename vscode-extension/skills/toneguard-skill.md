---
name: ToneGuard Writing Style
description: Avoid AI slop patterns in generated text. Follow these rules when writing prose, documentation, comments, or any natural language content.
version: 0.1.44
triggers:
  - writing
  - documentation
  - comments
  - markdown
  - readme
  - prose
---

# ToneGuard Writing Style Guide

When generating prose, documentation, or comments, follow these rules to avoid AI slop patterns.

## Banned Buzzwords (Never Use These)

Replace these with simpler, more direct alternatives:

| Avoid | Use Instead |
|-------|-------------|
| delve, delve into, deep dive | explore, examine, look at |
| leverage | use |
| utilize, utilise | use |
| facilitate | help, enable |
| optimize, optimise | improve |
| embark, embark on a journey | start, begin |
| robust | solid, reliable |
| innovative | new, novel |
| seamless, seamlessly | smooth, smoothly |
| groundbreaking | new, significant |
| holistic | complete, whole |
| paradigm, paradigm-shifting | model, approach |
| synergy | cooperation, combined effect |
| ecosystem | system, environment |
| landscape | field, area |
| realm | area, domain |
| tapestry | mix, combination |
| stakeholder | user, customer, team |
| empower | help, enable |
| scalable | flexible, expandable |
| transformative | significant, major |
| cutting-edge | modern, latest |
| game-changing | significant, important |
| next-generation | new, upcoming |
| revolutionary | new, significant |
| state-of-the-art | modern, current |
| unprecedented | unusual, rare |
| plethora | many, several |
| comprehensive | complete, thorough |
| streamlined | simplified, efficient |
| actionable insights | recommendations, findings |
| data-driven | based on data |
| best practices | recommended approach |

## Banned Puffery Phrases (Never Use)

These phrases are empty marketing speak:

- rich cultural heritage, vibrant cultural heritage
- cultural tapestry
- breathtaking, stunning natural beauty
- must-visit, must-see
- enduring legacy, lasting legacy
- nestled, in the heart of
- stands as a symbol of, stands as a testament
- plays a pivotal role in
- leaves a lasting impact
- hallmark of innovation
- gateway to
- thriving ecosystem, vibrant ecosystem
- groundbreaking innovation
- unparalleled excellence
- a seamless journey, a diverse tapestry
- ultimate solution, one-stop shop, all-in-one platform
- future-proof
- trusted by leading brands

## Banned Template Phrases (Never Start With)

Do not begin sentences or paragraphs with:

- "In conclusion..."
- "Overall..."
- "In summary..."
- "In essence..."
- "In today's fast-paced world..."
- "In today's ever-evolving world..."
- "Future prospects include..."
- "It is worth noting..."
- "It is important to note..."
- "It should be mentioned..."
- "One might argue..."
- "Based on the information provided..."
- "As technology continues to evolve..."
- "In an ever-evolving landscape..."
- "At the end of the day..."
- "Looking ahead..."
- "Key takeaways..."

## Banned AI Self-Reference Phrases

Never use these (they reveal AI authorship):

- "As an AI language model..."
- "As a language model..."
- "My knowledge cutoff..."
- "I don't have access to..."
- "I cannot access..."
- "I hope this helps!"
- "Let me know if you have any questions!"
- "Feel free to reach out!"

## Banned Weasel Words (Cite Sources Instead)

Instead of vague attributions, cite specific sources:

- ~~"Some critics argue"~~ → [Author Name] argues in [Source]
- ~~"Experts say"~~ → Dr. Smith states in [Paper]
- ~~"Studies show"~~ → A 2024 study by [Institution] found
- ~~"Research suggests"~~ → [Specific research] indicates
- ~~"Many experts believe"~~ → [Named experts] believe
- ~~"It is widely believed"~~ → According to [Source]

## Banned Marketing Clichés

- unlock the power of
- revolutionize the way
- take your business to the next level
- game-changing solution
- cutting-edge technology
- seamlessly integrated
- disruptive innovation
- seamless/delightful experience
- unlock your potential
- empower your [anything]
- limited time offer, don't miss out, act now

## Throttled Transitions (Use Sparingly - Max 1 Per Section)

These are fine occasionally but become slop when overused:

- furthermore, moreover
- consequently, thus, therefore
- accordingly, nonetheless
- subsequently, additionally
- in addition to, alongside this
- as a result, in fact
- in essence, in summary
- significantly, remarkably, notably

## Structure Rules

1. **Vary sentence starters** - Do not start 3+ consecutive sentences with the same word
2. **Limit em-dashes** - Maximum 1 em-dash (—) per paragraph
3. **Avoid rule of three** - Don't stack 3+ parallel items in lists repeatedly
4. **Vary sentence length** - Mix short and long sentences; avoid uniform 15-20 word sentences
5. **Limit exclamations** - Maximum 1 exclamation mark per paragraph
6. **Limit questions as leads** - Don't start multiple consecutive paragraphs with questions

## Prefer Specifics Over Generalities

Always prefer concrete details:

| Vague | Specific |
|-------|----------|
| "many users" | "1,200 users" or "~40% of users" |
| "significantly faster" | "3.2x faster" or "reduced from 800ms to 250ms" |
| "recently" | "in v2.3.1" or "as of January 2024" |
| "various improvements" | "fixed memory leak in parser, added retry logic" |
| "the function" | "`parseConfig()`" or "`UserService.authenticate()`" |
| "the file" | "`src/config.yml`" or "`package.json`" |

## Confidence Claims

- **Never claim percentages without evidence** - Don't say "100% accurate" or "99% uptime" without data
- **Cite sources for statistics** - "According to our test suite (847/847 tests passing)"
- **Use hedged language when uncertain** - "approximately", "around", "typically"
- **Avoid superlatives** - "best", "fastest", "most powerful" require proof

## Broad Terms to Avoid (Be Specific)

Instead of vague nouns, describe what you actually mean:

- ~~solution~~ → tool, library, API, service
- ~~platform~~ → web app, CLI, SDK
- ~~ecosystem~~ → community, toolchain, integrations
- ~~experience~~ → workflow, interface, process
- ~~framework~~ → library, architecture, pattern
- ~~journey~~ → process, workflow, onboarding
- ~~learnings~~ → lessons, findings, insights

## Examples

### Bad (AI Slop)

> In today's fast-paced digital landscape, leveraging cutting-edge solutions is crucial for seamless integration. Our groundbreaking platform empowers stakeholders to unlock unprecedented synergies and drive transformative outcomes. Furthermore, this holistic approach ensures scalable, robust performance.

### Good (Human-Like)

> The CLI parses `config.yml` and validates fields against the schema. If validation fails, it exits with code 1 and prints the first error. Run `dwg lint --fix` to auto-correct common issues like trailing whitespace.

---

**Remember**: Good technical writing is specific, direct, and evidence-based. Every claim should be verifiable. Every term should mean something concrete.
