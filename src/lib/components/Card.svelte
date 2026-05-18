<script lang="ts">
  type Variant = "default" | "subtle" | "highlighted";

  let {
    variant = "default",
    class: className = "",
    children,
  }: {
    variant?: Variant;
    class?: string;
    children?: import("svelte").Snippet;
  } = $props();

  const variants: Record<Variant, string> = {
    default: "rounded-md border bg-background shadow-sm transition-shadow",
    subtle: "rounded-lg border border-transparent bg-muted/50 hover:bg-muted/70 transition-colors",
    highlighted:
      "rounded-md border-2 border-primary/20 bg-background shadow-sm hover:border-primary/40 transition-all",
  };
</script>

<div class="{variants[variant]} {className}">
  {#if children}
    {@render children()}
  {/if}
</div>
