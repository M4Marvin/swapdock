import { createFileRoute } from '@tanstack/react-router'
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from '@/components/ui/accordion'
import { Container, PageHero, Section } from '@/components/section'
import { FinalCta } from '@/components/marketing/final-cta'
import { FAQ } from '@/content/faq'
import { jsonLd, pageHead } from '@/lib/seo'

export const Route = createFileRoute('/faq')({
  head: () =>
    pageHead({
      title: 'FAQ — swapdock',
      description:
        'Kubernetes? No. A PaaS? No. SQLite? Safe by rule. Production-ready? Not yet, and here is exactly what that means.',
      path: '/faq',
    }),
  component: Faq,
})

function Faq() {
  const faqJson = {
    '@context': 'https://schema.org',
    '@type': 'FAQPage',
    mainEntity: FAQ.map((item) => ({
      '@type': 'Question',
      name: item.q,
      acceptedAnswer: { '@type': 'Answer', text: item.a },
    })),
  }

  return (
    <>
      <PageHero
        eyebrow="Questions"
        title="Before you ask."
        lead="The honest answers, including the one about production readiness."
      />

      <Section>
        <Container>
          <div className="mx-auto max-w-3xl">
            <Accordion className="border-t border-border" defaultValue={[0]}>
              {FAQ.map((item, i) => (
                <AccordionItem key={item.q} value={i} className="border-b border-border">
                  <AccordionTrigger className="py-5 text-[14px] hover:no-underline">
                    {item.q}
                  </AccordionTrigger>
                  <AccordionContent className="pb-5">
                    <p className="max-w-2xl text-[13px] leading-relaxed text-muted-foreground">{item.a}</p>
                  </AccordionContent>
                </AccordionItem>
              ))}
            </Accordion>
          </div>
        </Container>
      </Section>

      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: jsonLd(faqJson) }}
      />

      <FinalCta title="One last question." lead="What would make your next deploy boring?" />
    </>
  )
}
