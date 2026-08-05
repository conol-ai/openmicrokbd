import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-45 [&_svg]:pointer-events-none [&_svg]:size-4",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary: "border border-border bg-background text-foreground hover:bg-muted",
        destructive: "bg-destructive text-white hover:bg-destructive/90",
        warning: "border border-warning/40 bg-warning/10 text-warning-foreground hover:bg-warning/20",
        ghost: "text-muted-foreground hover:bg-muted hover:text-foreground"
      },
      size: {
        default: "h-9 px-3.5",
        sm: "h-8 gap-1.5 px-2.5 text-xs",
        icon: "size-9 p-0"
      }
    },
    defaultVariants: { variant: "secondary", size: "default" }
  }
);

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants> & { asChild?: boolean };

export function Button({ className, variant, size, asChild, ...props }: ButtonProps) {
  const Component = asChild ? Slot.Root : "button";
  return <Component data-slot="button" className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}
